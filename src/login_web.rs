//! 内置浏览器登录：以“同程序 --embedded-login 子进程”方式弹出 WebView2 登录窗。
//!
//! 为什么不用线程直接开窗:实测(2026-09)在 eframe(winit) 主进程里,后台线程用 tao
//! `EventLoopBuilder::with_any_thread(true)` 虽能建出事件循环,但 `WindowBuilder::build`
//! 会与同进程的 winit 发生原生层冲突,导致整个进程静默崩溃(无 panic 输出)。
//! 因此把登录窗放进子进程:子进程没有 eframe/winit,其主线程就是 tao 的合法主线程
//! (与 wry 官方 cookies 示例完全等价的运行形态),天然避开全部跨线程/跨事件循环问题。
//! 登录成功后 Cookie 写入 cfg 同目录的 login_result.json,父进程轮询回填。子进程即使
//! 崩溃也绝不影响主程序的抢课引擎与界面。
use crate::engine::Cmd;
use crate::model::{AppData, Lvl, Tab};
use crate::net;
use eframe::egui;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

const POLL_MS: u64 = 500; // 登录状态轮询间隔
const WATCHER_TIMEOUT_S: u64 = 1800; // 父进程最多等子进程 30 分钟(用户磨蹭登录也够)

/// 诊断用:关键路径写文件(GUI 子系统程序无声崩溃/无控制台时也能定位)。
/// 写在 exe 同目录 login_trace.log;写失败(如目录只读)静默忽略,不影响运行。
pub(crate) fn trace(msg: &str) {
    use std::io::Write;
    let p = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join("login_trace.log")))
        .unwrap_or_else(|| std::path::PathBuf::from("login_trace.log"));
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
        let _ = writeln!(f, "[{}] {}", chrono::Local::now().format("%H:%M:%S%.3f"), msg);
    }
}

/// 子进程与父进程之间的结果文件协议。
#[derive(serde::Serialize, serde::Deserialize, Default, Clone)]
struct LoginResult {
    status: String, // ok / canceled / failed
    message: String,
    cookie: String,
    base: String,
    profile: String,
}

fn result_path(cfg_path: &str) -> String {
    let p = std::path::Path::new(cfg_path);
    let dir = p.parent().unwrap_or(std::path::Path::new("."));
    dir.join("login_result.json").to_string_lossy().into_owned()
}

fn write_result(rp: &str, status: &str, message: &str, cookie: &str, base: &str, profile: &str) {
    let r = LoginResult {
        status: status.to_string(),
        message: message.to_string(),
        cookie: cookie.to_string(),
        base: base.to_string(),
        profile: profile.to_string(),
    };
    if let Ok(s) = serde_json::to_string(&r) {
        let _ = std::fs::write(rp, s);
    }
    trace(&format!("child: wrote result file status={}", status));
}

// =====================================================================
// 父进程侧:拉起子进程 + 后台 watcher 回填
// =====================================================================

/// 打开内置浏览器登录(非阻塞)。等价于点击按钮的完整入口。
pub fn open_login_window(ctx: egui::Context, data: Arc<Mutex<AppData>>, tx: mpsc::Sender<Cmd>) {
    trace("parent: open_login_window enter");
    let cfg_path = {
        let mut g = data.lock().unwrap();
        if g.login_open {
            g.log(Lvl::Warn, "内置浏览器已在运行,请先到弹出的登录窗口完成操作。");
            ctx.request_repaint();
            return;
        }
        g.login_open = true;
        g.login_state = "正在启动内置浏览器…".to_string();
        g.cfg_path.clone()
    };
    let rp = result_path(&cfg_path);
    let _ = std::fs::remove_file(&rp);

    // 先落盘配置,保证子进程一定能读到 base/profile(首次运行可能从未保存过)
    {
        let snap = data.lock().unwrap().clone();
        crate::save_cfg(&snap, &cfg_path);
    }

    let exe = std::env::current_exe().unwrap_or_else(|_| "chd-course-grabber.exe".into());
    trace(&format!("parent: spawning child {} --embedded-login", exe.display()));
    let child = match std::process::Command::new(&exe)
        .arg(format!("--embedded-login={}", cfg_path))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            let mut g = data.lock().unwrap();
            g.login_open = false;
            g.login_state = format!("启动失败:无法拉起内置浏览器({})", e);
            g.log(Lvl::Err, format!("启动内置浏览器子进程失败:{}", e));
            ctx.request_repaint();
            return;
        }
    };
    trace("parent: child spawned, watcher thread start");

    let webdata = std::path::Path::new(&cfg_path)
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join(".chd_login_webdata");

    let ctx2 = ctx.clone();
    let data2 = data.clone();
    let tx2 = tx.clone();
    std::thread::spawn(move || {
        // 子进程收尾后尝试清理 WebView2 残留数据目录(尽力而为;失败则下次复用)
        let cleanup = |wd: &std::path::Path| {
            let _ = std::fs::remove_dir_all(wd);
        };
        let mut child = child;
        let deadline = Instant::now() + Duration::from_secs(WATCHER_TIMEOUT_S);
        loop {
            // 1) 结果文件出现 -> 回填
            if let Ok(txt) = std::fs::read_to_string(&rp) {
                if let Ok(r) = serde_json::from_str::<LoginResult>(&txt) {
                    trace(&format!("parent: result file -> {}", r.status));
                    finish_login(&ctx2, &data2, &tx2, &r, &cfg_path);
                    cleanup(&webdata);
                    return;
                }
            }
            // 2) 子进程已退出且仍无结果 -> 失败
            if let Ok(Some(_st)) = child.try_wait() {
                if let Ok(txt) = std::fs::read_to_string(&rp) {
                    if let Ok(r) = serde_json::from_str::<LoginResult>(&txt) {
                        trace(&format!("parent: late result -> {}", r.status));
                        finish_login(&ctx2, &data2, &tx2, &r, &cfg_path);
                        cleanup(&webdata);
                        return;
                    }
                }
                trace("parent: child exited with no result");
                let mut g = data2.lock().unwrap();
                g.login_open = false;
                g.login_state = "内置浏览器启动失败(窗口进程已退出)。若反复失败:① 确认已安装 WebView2 运行时;② 改用“方式 A 外部浏览器复制 Cookie”。".into();
                g.log(Lvl::Err, "内置浏览器窗口进程异常退出,未获得会话。");
                ctx2.request_repaint();
                cleanup(&webdata);
                return;
            }
            // 3) 看门狗:子进程僵死(既未退出也未写文件)则杀掉并复位
            if Instant::now() > deadline {
                let _ = child.kill();
                trace("parent: watchdog killed stuck child");
                let mut g = data2.lock().unwrap();
                g.login_open = false;
                g.login_state = "内置浏览器等待超时,已关闭。请重试。".into();
                g.log(Lvl::Warn, "内置浏览器登录超时(30 分钟)自动关闭。");
                ctx2.request_repaint();
                cleanup(&webdata);
                return;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    });
    ctx.request_repaint();
}

/// watcher 拿到结果后回填 AppData(只在状态真正可用时发拉课命令)。
fn finish_login(
    ctx: &egui::Context,
    data: &Arc<Mutex<AppData>>,
    tx: &mpsc::Sender<Cmd>,
    r: &LoginResult,
    cfg_path: &str,
) {
    // 状态更新统一在锁内完成,锁外再落盘/发命令,避免持锁或 drop 后借用。
    let mut ok = false;
    {
        let mut g = data.lock().unwrap();
        match r.status.as_str() {
            "ok" => {
                ok = true;
                g.cookie = r.cookie.clone();
                if !r.base.is_empty() && r.base != g.base {
                    g.log(Lvl::Info, format!("教务实际地址为 {},已自动切换。", r.base));
                    g.base = r.base.clone();
                }
                if !r.profile.is_empty() && r.profile != g.profile {
                    g.log(Lvl::Info, format!("批次已自动切换为 #{}.", r.profile));
                    g.profile = r.profile.clone();
                }
                g.session_msg = "✓ 内置浏览器登录成功,已自动获取会话 Cookie。可直接拉取课程。".into();
                g.login_state = "✓ 登录成功 · Cookie 已自动获取".into();
                g.log(Lvl::Ok, "内置浏览器登录成功,会话 Cookie 已保存。");
                if g.auto_fetch_after_login && g.tab == Tab::Session {
                    g.tab = Tab::Search;
                }
            }
            "canceled" => {
                if !g.cookie.trim().is_empty() {
                    g.login_state = "登录窗口已关闭(沿用已有 Cookie)".into();
                } else {
                    g.login_state = "已取消:未获取到新 Cookie".into();
                    g.log(Lvl::Warn, "内置浏览器被手动关闭,未获得会话。可再次点按钮重试,或粘贴 Cookie。");
                }
            }
            _ => {
                // failed
                g.login_state = if r.message.is_empty() {
                    "内置浏览器启动失败(详见日志)".into()
                } else {
                    format!("内置浏览器启动失败:{}", r.message)
                };
                g.log(Lvl::Err, format!("内置浏览器窗口报告失败:{}", r.message));
            }
        }
        g.login_open = false;
    }
    if ok {
        let snap = data.lock().unwrap().clone();
        crate::save_cfg(&snap, cfg_path);
        let _ = tx.send(Cmd::FetchCourses);
    }
    ctx.request_repaint();
}

// =====================================================================
// 子进程侧:主线程跑 tao 事件循环 + wry WebView2 登录窗(仅 --embedded-login 模式进入)
// =====================================================================

/// 子进程主线程入口(等价于 wry 官方示例的运行形态,主线程建窗,无任何跨线程问题)。
pub fn run_embedded_login_process(cfg_path: &str) {
    trace("child: run_embedded_login_process enter");
    use tao::dpi::LogicalSize;
    use tao::event::{Event, WindowEvent};
    use tao::event_loop::{ControlFlow, EventLoop};
    use tao::window::WindowBuilder;

    let rp = result_path(cfg_path);
    let _ = std::fs::remove_file(&rp);

    // 从父进程 cfg 读起始地址与批次(只取这两个字段,其余未知字段被 serde 忽略)
    #[derive(serde::Deserialize)]
    struct MiniCfg {
        base: String,
        profile: String,
    }
    let (base0, pid0) = match std::fs::read_to_string(cfg_path) {
        Ok(s) => match serde_json::from_str::<MiniCfg>(&s) {
            Ok(c) => (c.base, c.profile),
            Err(e) => {
                write_result(&rp, "failed", &format!("无法读取配置:{}", e), "", "", "");
                return;
            }
        },
        Err(e) => {
            write_result(&rp, "failed", &format!("无法打开配置文件:{}", e), "", "", "");
            return;
        }
    };
    let base_host = host_of(&base0).to_lowercase();
    let start_url = net::url_default(&base0, &pid0);
    trace(&format!("child: base={} profile={} start_url={}", base_host, pid0, start_url));

    // 子进程主线程建事件循环(不设 any_thread —— 这里就是主线程)
    let event_loop = EventLoop::new();
    trace("child: event loop ok");
    let window = match WindowBuilder::new()
        .with_title("内置浏览器登录 · 认证完成后本窗自动关闭")
        .with_inner_size(LogicalSize::new(1000.0, 780.0))
        .build(&event_loop)
    {
        Ok(w) => w,
        Err(e) => {
            write_result(&rp, "failed", &format!("创建窗口失败:{}", e), "", "", "");
            return;
        }
    };
    trace("child: window ok");
    // 独立 WebView2 数据目录:避免与同机器其他实例/崩溃残留进程争用默认目录
    // (Windows 上同 user data folder 被占用会报 ERROR_BUSY 0x800700AA)。放 cfg 旁,不写 C 盘。
    let webdata = std::path::Path::new(cfg_path)
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join(".chd_login_webdata");
    let mut wctx = wry::WebContext::new(Some(webdata.clone()));
    let webview = match wry::WebViewBuilder::new_with_web_context(&mut wctx)
        .with_url(&start_url)
        .build(&window)
    {
        Ok(w) => w,
        Err(e) => {
            write_result(&rp, "failed", &format!("WebView2 初始化失败:{}", e), "", "", "");
            return;
        }
    };
    trace("child: webview ok, entering run loop");

    let rp2 = rp.clone();
    let base_host2 = base_host.clone();
    let base0_2 = base0.clone();
    let pid0_2 = pid0.clone();
    let mut logged_in = false;

    // 子进程独立运行:抓到 Cookie 后写结果并退出;被关窗则写 canceled
    event_loop.run(move |event, _target, control_flow| {
        *control_flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(POLL_MS));
        match event {
            Event::NewEvents(_) => {
                if logged_in {
                    return;
                }
                match poll_login_once(&webview, &base_host2, &base0_2, &pid0_2) {
                    Some((cookie_str, base_eff, pid_eff)) => {
                        logged_in = true;
                        write_result(&rp2, "ok", "", &cookie_str, &base_eff, &pid_eff);
                        *control_flow = ControlFlow::Exit;
                    }
                    None => {}
                }
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                if !logged_in {
                    write_result(&rp2, "canceled", "登录窗口已手动关闭", "", "", "");
                }
                *control_flow = ControlFlow::Exit;
            }
            _ => {}
        }
    });
    // run() 返回 ! (内部 ExitProcess);理论上到不了这里
}

/// 单次检查:URL 是否已回到教务页。是 => 读取该站点 Cookie 并返回(cookie串, 实际base, 实际批次)。
fn poll_login_once(
    webview: &wry::WebView,
    base_host: &str,
    base0: &str,
    pid0: &str,
) -> Option<(String, String, String)> {
    let cur = webview.url().ok()?;
    if !looks_logged_in(&cur, base_host) {
        return None;
    }
    // 校准实际 base / 批次(登录跳转可能与配置略有差异)
    let mut base_eff = base0.to_string();
    let mut pid_eff = pid0.to_string();
    if let Ok(p) = url::Url::parse(&cur) {
        if let Some(h) = p.host_str() {
            base_eff = format!("{}://{}", p.scheme(), h);
        }
        if let Some(q) = p.query() {
            for key in ["electionProfile.id=", "profileId="] {
                if let Some(pos) = q.find(key) {
                    let v: String = q[pos + key.len()..]
                        .chars()
                        .take_while(|c| c.is_ascii_digit())
                        .collect();
                    if !v.is_empty() {
                        pid_eff = v;
                    }
                }
            }
        }
    }
    let cu = net::url_default(&base_eff, &pid_eff);
    let cookies = webview.cookies_for_url(&cu).ok()?;
    let mut map: std::collections::BTreeMap<String, String> = Default::default();
    for c in cookies {
        let name = c.name().to_string();
        let value = c.value().to_string();
        if !name.is_empty() {
            map.insert(name, value);
        }
    }
    if map.is_empty() {
        return None; // 空 cookie 视为尚未真正完成,继续等
    }
    let cookie_str = map
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join("; ");
    Some((cookie_str, base_eff, pid_eff))
}

/// 判断登录是否完成:URL 回到教务主机且路径在 /eams/ 下、不是登录/认证页。
fn looks_logged_in(cur: &str, base_host: &str) -> bool {
    let Ok(p) = url::Url::parse(cur) else { return false };
    let Some(h) = p.host_str() else { return false };
    let h = h.to_lowercase();
    if h != base_host && !h.ends_with(&format!(".{}", base_host)) {
        return false;
    }
    let path = p.path().to_lowercase();
    path.starts_with("/eams/") && !path.contains("login") && !path.contains("authserver")
}

fn host_of(u: &str) -> &str {
    let rest = u.split("://").nth(1).unwrap_or(u);
    rest.split(['/', '?']).next().unwrap_or(rest)
}
