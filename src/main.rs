#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod engine;
mod login_web;
mod model;
mod net;
mod sport;
mod style;

use engine::Cmd;
use model::{AppData, CourseRow, Lvl, Strategy, Tab, Target};
use eframe::egui;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

fn main() -> eframe::Result<()> {
    // 子进程模式:仅用于“内置浏览器登录”窗口(由父进程以 --embedded-login 拉起)。
    // 子进程没有 eframe/winit,主线程即 tao 主线程,可安全建窗;登录结果写入结果文件供父进程回填。
    for a in std::env::args().skip(1) {
        if let Some(cfg) = a.strip_prefix("--embedded-login=") {
            login_web::run_embedded_login_process(cfg);
            std::process::exit(0);
        }
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([1080.0, 700.0])
            .with_title(model::APP_TITLE),
        ..Default::default()
    };
    eframe::run_native(
        model::APP_TITLE,
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}

struct App {
    data: Arc<Mutex<AppData>>,
    tx: mpsc::Sender<Cmd>,
    run: Arc<AtomicBool>,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        style::setup(&cc.egui_ctx);
        let cfg_path = cfg_path();
        let mut d = AppData::default();
        d.cfg_path = cfg_path.clone();
        load_cfg(&mut d, &cfg_path);
        let auto_arm = d.sport_auto_arm;
        if auto_arm {
            d.tab = Tab::Sport; // 自动布防时默认停在体育抢课页
        }
        let data = Arc::new(Mutex::new(d));
        let (tx, run) = engine::spawn(cc.egui_ctx.clone(), data.clone());
        // 若配置了「启动自动启动体育引擎」:进引擎线程发一条启动命令(免手动点按钮)
        if auto_arm {
            let _ = tx.send(Cmd::SportArm);
        }
        App { data, tx, run }
    }

    fn send(&self, cmd: Cmd) {
        let _ = self.tx.send(cmd);
    }
}

fn cfg_path() -> String {
    if let Ok(p) = std::env::current_exe() {
        if let Some(dir) = p.parent() {
            return dir.join("chd_grabber_cfg.json").to_string_lossy().to_string();
        }
    }
    "chd_grabber_cfg.json".to_string()
}

#[derive(serde::Serialize, serde::Deserialize)]
struct CfgFile {
    base: String,
    profile: String,
    cookie: String,
    account: String,
    strategy: Strategy,
    refresh_ms: u64,
    submit_ms: u64,
    auto_stop_done: bool,
    submit_path: String,
    submit_query: String,
    submit_body: String,
    count_extra: String,
    data_path: String,
    #[serde(default = "def_sem")]
    semester_id: String,
    targets: Vec<Target>,
    #[serde(default = "def_true")]
    one_per_no: bool,
    #[serde(default = "def_true")]
    auto_fetch_after_login: bool,
    // 体育(stuh5)
    #[serde(default)]
    sport_accounts: Vec<model::SportAcc>,
    #[serde(default = "def_sport_fire")]
    sport_fire: String,
    #[serde(default = "def_sport_int")]
    sport_interval_ms: u64,
    #[serde(default)]
    sport_gate_fire: bool,
    #[serde(default)]
    sport_auto_arm: Option<bool>, // None=旧配置没有这个键 → 按 AppData 默认(true)处理
}

fn def_true() -> bool { true }
fn def_sem() -> String { String::new() }
fn def_sport_fire() -> String { "21:00".to_string() }
fn def_sport_int() -> u64 { 600 }

fn save_cfg(d: &AppData, path: &str) {
    let cfg = CfgFile {
        base: d.base.clone(),
        profile: d.profile.clone(),
        cookie: d.cookie.clone(),
        account: d.account.clone(),
        strategy: d.strategy,
        refresh_ms: d.refresh_ms,
        submit_ms: d.submit_ms,
        auto_stop_done: d.auto_stop_done,
        submit_path: d.submit_path.clone(),
        submit_query: d.submit_query.clone(),
        submit_body: d.submit_body.clone(),
        count_extra: d.count_extra.clone(),
        data_path: d.data_path.clone(),
        semester_id: d.semester_id.clone(),
        targets: d.targets.clone(),
        one_per_no: d.one_per_no,
        auto_fetch_after_login: d.auto_fetch_after_login,
        sport_accounts: d.sport_accounts.clone(),
        sport_fire: d.sport_fire.clone(),
        sport_interval_ms: d.sport_interval_ms,
        sport_gate_fire: d.sport_gate_fire,
        sport_auto_arm: Some(d.sport_auto_arm),
    };
    if let Ok(s) = serde_json::to_string_pretty(&cfg) {
        let _ = std::fs::write(path, s);
    }
}

fn load_cfg(d: &mut AppData, path: &str) {
    let Ok(s) = std::fs::read_to_string(path) else { return };
    let Ok(cfg) = serde_json::from_str::<CfgFile>(&s) else { return };
    d.base = cfg.base;
    d.profile = cfg.profile;
    d.cookie = cfg.cookie;
    d.account = cfg.account;
    d.strategy = cfg.strategy;
    d.refresh_ms = cfg.refresh_ms;
    d.submit_ms = cfg.submit_ms;
    d.auto_stop_done = cfg.auto_stop_done;
    d.submit_path = cfg.submit_path;
    d.submit_query = cfg.submit_query;
    d.submit_body = cfg.submit_body;
    d.count_extra = cfg.count_extra;
    d.data_path = cfg.data_path;
    d.semester_id = cfg.semester_id;
    d.targets = cfg.targets;
    d.one_per_no = cfg.one_per_no;
    d.auto_fetch_after_login = cfg.auto_fetch_after_login;
    // 体育字段:旧配置可能没有,保留代码默认值
    if !cfg.sport_accounts.is_empty() {
        d.sport_accounts = cfg.sport_accounts;
        for a in d.sport_accounts.iter_mut() {
            if a.st.is_empty() { a.st = "idle".into(); }
        }
    }
    if !cfg.sport_fire.is_empty() { d.sport_fire = cfg.sport_fire; }
    if cfg.sport_interval_ms > 0 { d.sport_interval_ms = cfg.sport_interval_ms; }
    d.sport_gate_fire = cfg.sport_gate_fire;
    if let Some(v) = cfg.sport_auto_arm { d.sport_auto_arm = v; }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 处理"打开内置浏览器登录"请求:按钮在渲染期间持着 data 锁,只能置标志;
        // 打开动作必须在锁外执行(open_login_window 内部会再次 lock,持锁期间调用=同线程重入死锁=界面未响应)。
        let need_open_login = {
            let mut g = self.data.lock().unwrap();
            let r = g.pending_open_login;
            g.pending_open_login = false;
            r
        };
        if need_open_login {
            let c2 = ctx.clone();
            let data = self.data.clone();
            let tx = self.tx.clone();
            login_web::open_login_window(c2, data, tx);
        }
        let mut d = self.data.lock().unwrap();
        // [无头诊断钩子] 模拟真实按钮点击:AUTO_CLICK_LOGIN=1 时首帧在持锁期间置
        // pending(与按钮代码完全相同),用于离线验证"按钮路径"不死锁、登录窗能弹出。
        {
            use std::sync::atomic::{AtomicBool, Ordering};
            static TRIGGERED: AtomicBool = AtomicBool::new(false);
            if !TRIGGERED.swap(true, Ordering::SeqCst)
                && std::env::var("AUTO_CLICK_LOGIN").as_deref() == Ok("1")
                && !d.login_open
            {
                d.pending_open_login = true;
                d.login_state = "正在启动内置浏览器…".to_string();
            }
        }
        if d.running || d.sport_active {
            ctx.request_repaint_after(Duration::from_millis(120));
        }
        let dark = ctx.style().visuals.dark_mode;
        let p = style::palette(dark);

        // ===================== 顶栏 =====================
        egui::TopBottomPanel::top("hdr")
            .exact_height(56.0)
            .frame(egui::Frame::none().fill(p.head_bg).inner_margin(egui::Margin::symmetric(18.0, 0.0)))
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.label(egui::RichText::new("CG").size(21.0).strong().color(if dark { p.accent } else { p.head_fg }));
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new("选课抢课助手 · URP/eams").size(15.0).strong().color(p.head_fg));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let (txt, c) = if d.running { ("● 抢课运行中", p.succ) } else { ("○ 空闲", p.dim) };
                        ui.label(egui::RichText::new(txt).size(13.0).strong().color(c));
                        ui.add_space(12.0);
                        ui.label(egui::RichText::new(format!("批次 #{}", d.profile)).size(12.0).color(p.head_fg));
                        ui.add_space(12.0);
                        let ok = !d.cookie.trim().is_empty();
                        let (t2, c2) = if ok { ("已配置会话", p.ok) } else { ("未配置 Cookie", p.warn) };
                        ui.label(egui::RichText::new(t2).size(12.0).color(c2));
                    });
                });
            });

        // ===================== 左侧导航 =====================
        egui::SidePanel::left("nav")
            .exact_width(172.0)
            .frame(egui::Frame::none().fill(p.panel).inner_margin(egui::Margin::same(10.0)))
            .show(ctx, |ui| {
                ui.add_space(8.0);
                let items = [
                    ("会话登录", Tab::Session),
                    ("课程检索", Tab::Search),
                    ("抢课中心", Tab::Center),
                    ("体育抢课", Tab::Sport),
                    ("运行日志", Tab::Log),
                    ("高级 · 模板", Tab::Advanced),
                ];
                for (txt, tab) in items {
                    let sel = tab == d.tab;
                    let (fill, fg) = if sel { (p.accent, p.accent_text) } else { (egui::Color32::TRANSPARENT, p.text) };
                    let r = egui::Frame::none()
                        .fill(fill)
                        .rounding(egui::Rounding::same(9.0))
                        .inner_margin(egui::Margin::symmetric(12.0, 9.0))
                        .show(ui, |ui| {
                            ui.set_width(132.0);
                            ui.label(egui::RichText::new(txt).size(14.0).strong().color(fg));
                        });
                    if r.response.interact(egui::Sense::click()).clicked() {
                        d.tab = tab;
                    }
                    ui.add_space(2.0);
                }
                ui.add_space(14.0);
                ui.label(egui::RichText::new("仅操作你自己的账号\n请在教务处规则允许内使用\n合理间隔,勿攻击服务器").size(11.0).color(p.dim));
            });

        // ===================== 中央区 =====================
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(p.bg).inner_margin(egui::Margin::same(16.0)))
            .show(ctx, |ui| {
                match d.tab {
                    Tab::Session => page_session(ui, &mut d, self),
                    Tab::Search => page_search(ui, &mut d, self),
                    Tab::Center => page_center(ui, &mut d, self),
                    Tab::Sport => page_sport(ui, &mut d, self),
                    Tab::Log => page_log(ui, &mut d),
                    Tab::Advanced => page_advanced(ui, &mut d),
                }
            });
    }
}

// ---------------------------------------------------------------------------
// 会话页
// ---------------------------------------------------------------------------
fn page_session(ui: &mut egui::Ui, d: &mut AppData, app: &App) {
    let p = style::palette_cur(ui);
    ui.horizontal(|ui| {
        ui.heading(egui::RichText::new("会话登录").size(20.0).strong());
        if !d.session_msg.is_empty() {
            let ok = d.session_msg.starts_with('✓');
            let c = if ok { p.ok } else { p.err };
            style::chip(ui, &d.session_msg, c, style::tint(c, 22));
        }
    });
    ui.add_space(2.0);
    ui.label(egui::RichText::new("登录在校方统一身份认证(CAS)完成。推荐用内置浏览器登录:Cookie 自动抓取,无需复制粘贴;也保留手动 Cookie 与自动登录两种方式。程序不保存你的密码。").size(12.5).color(p.dim));
    ui.add_space(10.0);

    // ---- 内置浏览器登录(推荐) ----
    style::card(ui, "内置浏览器一键登录(推荐)", |ui| {
        ui.horizontal(|ui| {
            if !d.login_open {
                if style::accent_button(ui, "🌐 打开内置浏览器登录").clicked() {
                    // 不在此直接打开:当前持锁(渲染中),open_login_window 内部会再取锁导致死锁。
                    // 只记录请求,由 update 下一帧在持锁前执行。
                    d.pending_open_login = true;
                    d.login_state = "正在启动内置浏览器…".to_string();
                }
            } else {
                ui.label(egui::RichText::new("登录窗口已打开…").size(14.0).strong().color(p.warn));
            }
            ui.checkbox(&mut d.auto_fetch_after_login, "登录成功后自动拉取本批次课程并跳到检索页");
        });
        if !d.login_state.is_empty() && d.login_state != "空闲" {
            ui.add_space(4.0);
            let ok = d.login_state.starts_with('✓');
            let c = if ok { p.ok } else if d.login_state.contains("失败") { p.err } else { p.info };
            style::chip(ui, &d.login_state, c, style::tint(c, 22));
        }
        ui.add_space(6.0);
        ui.label(egui::RichText::new("点按钮会弹出程序内置的登录窗口(系统 WebView2 内核):照常输入学号/密码,若需验证码/动态码也在窗口内完成;认证一通过,会话 Cookie(含 JSESSIONID)自动读入并保存,免去 F12 复制。").size(12.5).color(p.dim));
    });

    ui.add_space(10.0);
    style::card(ui, "方式 A · 外部浏览器复制 Cookie(备用)", |ui| {
        ui.label(egui::RichText::new("① 点下方按钮,浏览器打开选课页(会自动跳统一身份认证,输入学号密码;若需要验证码/动态码也照常操作):").size(13.0));
        ui.horizontal(|ui| {
            if style::soft_button(ui, "在浏览器中打开选课页").clicked() {
                let url = format!("{}/eams/stdElectCourse!defaultPage.action?electionProfile.id={}", d.base, d.profile);
                std::thread::spawn(move || { let _ = webbrowser::open(&url); });
            }
        });
        ui.add_space(4.0);
        ui.label(egui::RichText::new("② 到课程列表页后 F12 → Network,点任意请求,复制 Request Headers 里 Cookie: 后面的整串;").size(13.0));
        ui.label(egui::RichText::new("③ 粘贴到下面 →「测试会话」→ 去「课程检索」拉列表。").size(13.0));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if style::soft_button(ui, if d.cookie_visible { "隐藏 Cookie" } else { "显示 / 编辑" }).clicked() {
                d.cookie_visible = !d.cookie_visible;
            }
            if style::soft_button(ui, "清空").clicked() { d.cookie.clear(); d.session_msg.clear(); }
            if style::soft_button(ui, "保存设置").clicked() { save_cfg(d, &d.cfg_path); d.log(Lvl::Info, "设置已保存。"); }
        });
        ui.add_space(4.0);
        if d.cookie_visible {
            ui.add(egui::TextEdit::multiline(&mut d.cookie)
                .hint_text("JSESSIONID=xxx; route=xxx; …")
                .desired_rows(3).desired_width(f32::INFINITY).font(egui::TextStyle::Monospace));
        } else if !d.cookie.is_empty() {
            let masked: String = d.cookie.chars().map(|c| if c.is_ascii_whitespace() || c == ';' || c == '=' { c } else { '•' }).collect();
            ui.add(egui::Label::new(egui::RichText::new(masked).monospace().color(p.dim)).wrap());
        } else {
            ui.label(egui::RichText::new("(Cookie 为空 —— 按上面步骤粘贴后会自动出现在这里)").size(12.5).color(p.dim));
        }
        ui.add_space(8.0);
        if style::accent_button(ui, "测试会话").clicked() {
            app.send(Cmd::TestSession);
        }
    });

    ui.add_space(10.0);
    style::card(ui, "方式 B · 自动登录(登录页无验证码时可用)", |ui| {
        ui.label(egui::RichText::new("部分学校 authserver 登录页没有验证码/动态码,可直接提交;有验证码/动态码时会提示改用方式 A。密码仅本次内存使用,不会写入磁盘。").size(12.5).color(p.dim));
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label("学号");
            ui.add(egui::TextEdit::singleline(&mut d.account).desired_width(150.0));
            ui.label("密码");
            ui.add(egui::TextEdit::singleline(&mut d.password).password(true).desired_width(160.0)
                .hint_text("初始:以学校通知为准"));
            if style::accent_button(ui, "自动登录").clicked() {
                app.send(Cmd::AutoLogin);
            }
        });
    });

    ui.add_space(10.0);
    style::card(ui, "选课批次(electionProfile.id) — 每次开放/关闭都会变,务必点“自动发现轮次”", |ui| {
        ui.horizontal(|ui| {
            ui.label("教务地址");
            ui.add(egui::TextEdit::singleline(&mut d.base).desired_width(240.0).font(egui::TextStyle::Monospace));
            ui.label("批次编号");
            ui.add(egui::TextEdit::singleline(&mut d.profile).desired_width(64.0));
            ui.label("semesterId");
            ui.add(egui::TextEdit::singleline(&mut d.semester_id).desired_width(52.0));
        });
        ui.horizontal(|ui| {
            if style::accent_button(ui, "🔄 自动发现轮次").clicked() { app.send(Cmd::FetchElections); }
            if style::soft_button(ui, "🌐 浏览器打开选课首页").clicked() {
                let url = format!("{}/eams/stdElectCourse.action", d.base);
                std::thread::spawn(move || { let _ = webbrowser::open(&url); });
            }
            ui.label(egui::RichText::new("semesterId 拉课程成功时会自动提取;抢课(余量显示)需要它。").size(11.5).color(p.dim));
        });
        if !d.elections.is_empty() {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("发现的可选轮次:");
                egui::ComboBox::from_id_salt("sel_election")
                    .selected_text(format!("#{} {}", d.profile, current_election_name(d)))
                    .width(340.0)
                    .show_ui(ui, |ui| {
                        for (name, id) in d.elections.iter() {
                            ui.selectable_value(&mut d.profile, id.clone(), format!("{}  (#{})", name, id));
                        }
                    });
                if style::soft_button(ui, "应用该轮次").clicked() {
                    save_cfg(d, &d.cfg_path);
                    d.log(Lvl::Ok, format!("轮次已切换为 #{} → 去「课程检索」拉取课程", d.profile));
                }
            });
        } else {
            ui.add_space(6.0);
            ui.label(egui::RichText::new("点“自动发现轮次”:程序去选课首页把你当前可进入的轮次都列出来,自动选中第一个。手工编号=网址里 electionProfile.id= 后的数字。").size(12.0).color(p.dim));
        }
    });
}

fn current_election_name(d: &AppData) -> String {
    d.elections.iter().find(|(_, id)| *id == d.profile).map(|(n, _)| n.clone()).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// 检索页
// ---------------------------------------------------------------------------
fn page_search(ui: &mut egui::Ui, d: &mut AppData, app: &App) {
    let p = style::palette_cur(ui);
    ui.horizontal(|ui| {
        ui.heading(egui::RichText::new("课程检索").size(20.0).strong());
        ui.add_space(6.0);
        ui.label(egui::RichText::new("关键词命中课程/教学班 → 一键加入抢课目标").size(13.0).color(p.dim));
    });
    ui.add_space(8.0);

    style::card(ui, "", |ui| {
        ui.horizontal(|ui| {
            ui.label("关键词");
            ui.add(egui::TextEdit::singleline(&mut d.keyword).desired_width(260.0)
                .hint_text("空格分隔:匹配 课程名/编号/教师"));
            if style::accent_button(ui, "拉取课程列表").clicked() { app.send(Cmd::FetchCourses); }
            if style::soft_button(ui, "🔍 仅探测 Lesson").clicked() { app.send(Cmd::ProbeLessons); }
            let rows = shown_courses(d);
            ui.label(egui::RichText::new(format!("命中 {} / 共 {} 个教学班", rows.len(), d.courses.len()))
                .size(12.0).color(p.dim));
            if !rows.is_empty() {
                let dup_n = d.targets.len();
                let btn = egui::Button::new(egui::RichText::new("批量加入目标").size(12.5).strong().color(p.text))
                    .fill(p.card_alt).stroke(egui::Stroke::new(1.0, p.border));
                if ui.add(btn).clicked() {
                    add_to_targets(d, &rows);
                }
                let _ = dup_n;
            }
        });
    });

    ui.add_space(8.0);
    style::card(ui, "⚡ 快捷自动抢 · 关键词直达(无需先加目标)", |ui| {
        ui.horizontal(|ui| {
            ui.label("课程关键词");
            let resp = ui.add(egui::TextEdit::singleline(&mut d.quick_kw).desired_width(260.0)
                .hint_text("如:大学物理 / 高数A / 课程编号"));
            let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if style::accent_button(ui, "猛攻直抢(先到先得)").clicked() || enter {
                let kw = d.quick_kw.clone();
                app.send(Cmd::QuickGrab(Strategy::Rush, kw));
            }
            if style::soft_button(ui, "蹲守抢漏(有余才抢)").clicked() {
                let kw = d.quick_kw.clone();
                app.send(Cmd::QuickGrab(Strategy::Free, kw));
            }
        });
        ui.add_space(2.0);
        ui.label(egui::RichText::new("命中关键词的课程会全部自动加入目标并立即开抢;同课程编号抢到一门后,其余教学班自动停用(可在“抢课中心”关闭该行为)。输入后按回车 = 猛攻直抢。")
            .size(12.0).color(p.dim));
    });

    ui.add_space(10.0);
    let rows = shown_courses(d);
    if rows.is_empty() {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            ui.label(egui::RichText::new("暂无课程数据。先填好关键词再点「拉取课程列表」。").size(14.0).color(p.dim));
            ui.label(egui::RichText::new("解析出若干教学班后,点行尾 ＋ 加入“抢课中心”。").size(13.0).color(p.dim));
        });
        return;
    }

    egui::ScrollArea::vertical().id_salt("courselist").auto_shrink([false, false]).show(ui, |ui| {
        for (i, c) in rows.iter().take(600).enumerate() {
            let bg = if i % 2 == 0 { p.card } else { p.card_alt };
            egui::Frame::none().fill(bg).rounding(egui::Rounding::same(8.0))
                .inner_margin(egui::Margin::symmetric(10.0, 5.0)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let (txt, cc) = match (c.sc, c.lc) {
                        (Some(s), Some(l)) if s < l => ("有空位", p.ok),
                        (Some(_), Some(_)) => ("已满", p.warn),
                        _ => ("未同步", p.dim),
                    };
                    style::chip(ui, txt, cc, style::tint(cc, 22));
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new(&c.no).size(12.5).monospace());
                    ui.label(egui::RichText::new(&c.name).size(13.5).strong());
                    ui.add_space(10.0);
                    ui.label(egui::RichText::new(&c.teachers).size(12.5).color(p.dim));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let cap = match (c.sc, c.lc) {
                            (Some(s), Some(l)) => format!("{} / {}", s, l),
                            _ => "—/—".to_string(),
                        };
                        ui.label(egui::RichText::new(cap).size(12.5).monospace().color(p.text));
                        ui.add_space(14.0);
                        let dup = d.targets.iter().any(|t| t.id == c.id);
                        let btn = egui::Button::new(egui::RichText::new(if dup { "已在目标" } else { "＋ 加入" })
                            .size(12.5).strong().color(if dup { p.dim } else { p.accent_text }))
                            .fill(if dup { egui::Color32::TRANSPARENT } else { p.accent });
                        if dup {
                            ui.add(btn);
                        } else if ui.add(btn).clicked() {
                            d.targets.push(Target::new(c.id, &c.no, &c.name));
                            d.log(Lvl::Ok, format!("加入目标:[{}] {} #{}", c.no, c.name, c.id));
                            save_cfg(d, &d.cfg_path);
                        }
                    });
                });
            });
            ui.add_space(3.0);
        }
        if rows.len() > 600 {
            ui.label(egui::RichText::new(format!("… 还有 {} 条,请用更精确的关键词缩小范围", rows.len() - 600)).size(12.0).color(p.dim));
        }
    });
}

fn shown_courses(d: &AppData) -> Vec<CourseRow> {
    let kws: Vec<String> = d.keyword
        .split(|c: char| c == ' ' || c == '、' || c == ',' || c == '，')
        .map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect();
    d.courses.iter().filter(|c| {
        if kws.is_empty() { return true; }
        let hay = format!("{} {} {}", c.no.to_lowercase(), c.name.to_lowercase(), c.teachers.to_lowercase());
        kws.iter().all(|k| hay.contains(k.as_str()))
    }).cloned().collect()
}

/// 把一批教学班加入抢课目标(自动去重),并保存配置。
fn add_to_targets(d: &mut AppData, rows: &[CourseRow]) {
    let mut added = 0usize;
    let mut dup = 0usize;
    for c in rows {
        if d.targets.iter().any(|t| t.id == c.id) { dup += 1; continue; }
        d.targets.push(Target::new(c.id, &c.no, &c.name));
        added += 1;
    }
    if added == 0 {
        d.log(Lvl::Info, format!("这些教学班都已在目标列表(重复 {} 个),无需重复加入。", dup));
    } else {
        d.log(Lvl::Ok, format!("已加入 {} 个目标(跳过重复 {} 个)。可到“抢课中心”开始。", added, dup));
        save_cfg(d, &d.cfg_path);
    }
}

// ---------------------------------------------------------------------------
// 抢课中心
// ---------------------------------------------------------------------------
#[derive(Clone, Copy)]
enum TargetAct { Up, Down, Del }

fn page_center(ui: &mut egui::Ui, d: &mut AppData, app: &App) {
    let p = style::palette_cur(ui);
    ui.horizontal(|ui| {
        ui.heading(egui::RichText::new("抢课中心").size(20.0).strong());
        if d.running {
            style::chip(ui, &d.engine_label, p.succ, style::tint(p.succ, 22));
        }
    });
    ui.add_space(8.0);

    // ---- 目标列表 ----
    egui::Frame::none().fill(p.card).stroke(egui::Stroke::new(1.0, p.border))
        .rounding(egui::Rounding::same(12.0)).inner_margin(egui::Margin::same(12.0))
        .show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("抢课目标({})", d.targets.len())).size(15.0).strong().color(p.accent));
            ui.add_space(10.0);
            if style::soft_button(ui, "全启用").clicked() {
                for t in d.targets.iter_mut() { t.enabled = true; }
                save_cfg(d, &d.cfg_path);
            }
            if style::soft_button(ui, "全停用").clicked() {
                for t in d.targets.iter_mut() { t.enabled = false; }
                save_cfg(d, &d.cfg_path);
            }
        });
        ui.add_space(6.0);
        if d.targets.is_empty() {
            ui.label(egui::RichText::new("目标为空 —— 去「课程检索」把要抢的教学班加进来。").size(13.0).color(p.dim));
        } else {
            egui::ScrollArea::vertical().id_salt("targets").max_height(300.0).auto_shrink([false, false]).show(ui, |ui| {
                let mut act: Option<(usize, TargetAct)> = None;
                let mut dirty = false;
                for i in 0..d.targets.len() {
                    let t = d.targets[i].clone();
                    let bg = if i % 2 == 0 { p.card } else { p.card_alt };
                    egui::Frame::none().fill(bg).rounding(egui::Rounding::same(8.0))
                        .inner_margin(egui::Margin::symmetric(8.0, 4.0)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            if ui.checkbox(&mut d.targets[i].enabled, "").on_hover_text("启停").changed() {
                                dirty = true;
                            }
                            ui.label(egui::RichText::new(if t.no.is_empty() { t.id.to_string() } else { format!("{} #{}", t.no, t.id) })
                                .size(12.0).monospace().color(p.dim));
                            ui.add_space(8.0);
                            ui.label(egui::RichText::new(&t.name).size(13.5).strong());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let cap = match (t.sc, t.lc) {
                                    (Some(a), Some(b)) => format!("{}/{}", a, b),
                                    _ => String::new(),
                                };
                                if !cap.is_empty() {
                                    ui.label(egui::RichText::new(cap).size(11.5).monospace().color(p.dim));
                                    ui.add_space(6.0);
                                }
                                let c = style::state_color(&t.state, ui.ctx().style().visuals.dark_mode);
                                style::chip(ui, style::state_label(&t.state), c, style::tint(c, 24));
                                ui.add_space(4.0);
                                if t.tries > 0 {
                                    ui.label(egui::RichText::new(format!("{} 次", t.tries)).size(11.0).color(p.dim));
                                }
                                if !t.last.is_empty() {
                                    ui.label(egui::RichText::new(&t.last).size(11.0).color(p.dim));
                                }
                                ui.add_space(6.0);
                                if ui.small_button("▲").on_hover_text("上移").clicked() { act = Some((i, TargetAct::Up)); }
                                if ui.small_button("▼").on_hover_text("下移").clicked() { act = Some((i, TargetAct::Down)); }
                                if ui.small_button("✕").on_hover_text("移除").clicked() { act = Some((i, TargetAct::Del)); }
                            });
                        });
                    });
                    if dirty {
                        save_cfg(d, &d.cfg_path);
                    }
                    ui.add_space(2.0);
                }
                if let Some((i, a)) = act {
                    match a {
                        TargetAct::Up if i > 0 => d.targets.swap(i, i - 1),
                        TargetAct::Down if i + 1 < d.targets.len() => d.targets.swap(i, i + 1),
                        TargetAct::Del => { d.targets.remove(i); }
                        _ => {}
                    }
                    save_cfg(d, &d.cfg_path);
                }
            });
        }
    });

    ui.add_space(10.0);
    // ---- 控制台 ----
    egui::Frame::none().fill(p.card).stroke(egui::Stroke::new(1.0, p.border))
        .rounding(egui::Rounding::same(12.0)).inner_margin(egui::Margin::same(14.0))
        .show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("策略与节奏").size(15.0).strong().color(p.accent));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !d.running {
                    if style::accent_button(ui, "▶  开始").clicked() {
                        save_cfg(d, &d.cfg_path);
                        app.send(Cmd::Start);
                    }
                } else if style::soft_button(ui, "■  停止").clicked() {
                    app.run.store(false, Ordering::SeqCst);
                }
                ui.checkbox(&mut d.auto_stop_done, "全部抢中后自动停止");
            });
        });
        ui.horizontal(|ui| {
            ui.checkbox(&mut d.one_per_no, "同课程编号抢到一门后,自动停用其余教学班(防重复/冲突)");
            if style::soft_button(ui, "保存设置").clicked() { save_cfg(d, &d.cfg_path); d.log(Lvl::Info, "设置已保存。"); }
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label("模式");
            ui.selectable_value(&mut d.strategy, Strategy::Free, Strategy::Free.label());
            ui.selectable_value(&mut d.strategy, Strategy::Rush, Strategy::Rush.label());
            ui.selectable_value(&mut d.strategy, Strategy::Watch, Strategy::Watch.label());
        });
        ui.label(egui::RichText::new(d.strategy.desc()).size(12.0).color(p.dim));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label("余量刷新间隔");
            ui.add(egui::DragValue::new(&mut d.refresh_ms).speed(100.0).range(150.0..=30000.0).suffix(" ms"));
            for s in [500u64, 1000, 1500, 3000, 8000] {
                if style::soft_button(ui, &format!("{}", s)).clicked() { d.refresh_ms = s; }
            }
        });
        ui.horizontal(|ui| {
            ui.label("提交间隔");
            ui.add(egui::DragValue::new(&mut d.submit_ms).speed(30.0).range(60.0..=5000.0).suffix(" ms"));
            for s in [100u64, 200, 400, 800, 2000] {
                if style::soft_button(ui, &format!("{}", s)).clicked() { d.submit_ms = s; }
            }
            ui.label(egui::RichText::new("推荐 ≥200ms,太快要防被风控也请别给学校服务器添堵").size(11.5).color(p.dim));
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label("一档预设");
            if style::soft_button(ui, "开抢(直选开放瞬间)").clicked() {
                d.strategy = Strategy::Rush; d.refresh_ms = 800; d.submit_ms = 250;
            }
            if style::soft_button(ui, "蹲守捡漏(退课高发期)").clicked() {
                d.strategy = Strategy::Free; d.refresh_ms = 1000; d.submit_ms = 700;
            }
            if style::soft_button(ui, "安静蹲守(仅监控)").clicked() {
                d.strategy = Strategy::Watch; d.refresh_ms = 1500; d.submit_ms = 800;
            }
            if style::soft_button(ui, "保存设置").clicked() { save_cfg(d, &d.cfg_path); }
        });
    });
    ui.add_space(8.0);
    ui.label(egui::RichText::new("运行中可随时改模式/间隔,下一轮自动生效;空位与抢中会实时高亮。最终结果以教务「已选课程」页为准。")
        .size(12.0).color(p.dim));
}

// ---------------------------------------------------------------------------
// 体育抢课页(stuh5 智慧体育)
// ---------------------------------------------------------------------------
fn sport_st_color(st: &str, p: &style::Palette) -> egui::Color32 {
    match st {
        "armed" => p.info,
        "run" => p.warn,
        "ok" => p.succ,
        "err" => p.err,
        "fail" => p.dim,
        _ => p.dim,
    }
}

fn page_sport(ui: &mut egui::Ui, d: &mut AppData, app: &App) {
    let p = style::palette_cur(ui);
    ui.horizontal(|ui| {
        ui.heading(egui::RichText::new("体育抢课 · 智慧选课(stuh5)").size(20.0).strong());
        if d.sport_active {
            style::chip(ui, &d.sport_msg, p.succ, style::tint(p.succ, 22));
        } else if d.sport_msg != "空闲" {
            style::chip(ui, &d.sport_msg, p.dim, style::tint(p.dim, 18));
        }
    });
    ui.add_space(2.0);
    ui.label(egui::RichText::new("每个体育账号填自己的 token + 志愿班级 id。启动引擎后:未入册的号按志愿序立即抢/到点抢,已入册的号盯防——一旦记录被系统清掉自动抢回,到开抢时刻再发一次通道确认。公众号登录后 F12→Console 输入 localStorage.getItem('ACCESS_TOKEN') 取 token。")
        .size(12.5).color(p.dim));
    ui.add_space(8.0);

    egui::ScrollArea::vertical().id_salt("sport_page").auto_shrink([false, false]).show(ui, |ui| {
        style::card(ui, "① 全局控制", |ui| {
            ui.horizontal(|ui| {
                if style::soft_button(ui, "载入课程库(任一token)").clicked() {
                    app.send(Cmd::SportLoad);
                }
                if style::soft_button(ui, "验证全部 token").clicked() {
                    app.send(Cmd::SportVerify);
                }
                ui.add_space(8.0);
                if !d.sport_active {
                    if style::accent_button(ui, "▶ 启动引擎(盯防 / 到点抢)").clicked() {
                        save_cfg(d, &d.cfg_path);
                        app.send(Cmd::SportArm);
                    }
                } else {
                    if style::soft_button(ui, "■ 停止引擎").clicked() {
                        app.send(Cmd::SportStop);
                    }
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("开抢时刻");
                ui.add(egui::TextEdit::singleline(&mut d.sport_fire).desired_width(58.0)
                    .hint_text("21:00").font(egui::TextStyle::Monospace));
                for t in ["21:00", "21:30", "22:00", "22:30"] {
                    if style::soft_button(ui, t).clicked() { d.sport_fire = t.to_string(); }
                }
                ui.add_space(10.0);
                ui.label("重试间隔");
                ui.add(egui::DragValue::new(&mut d.sport_interval_ms).speed(50.0).range(200.0..=5000.0).suffix(" ms"));
                for s in [400u64, 600, 1000, 2000] {
                    if style::soft_button(ui, &format!("{}", s)).clicked() { d.sport_interval_ms = s; }
                }
            });
            ui.horizontal(|ui| {
                ui.checkbox(&mut d.sport_auto_arm, "启动程序时自动启动引擎(免手动点启动,默认开)");
                if style::soft_button(ui, "保存设置").clicked() { save_cfg(d, &d.cfg_path); d.log(Lvl::Info, "体育配置已保存。"); }
            });
            ui.horizontal(|ui| {
                ui.checkbox(&mut d.sport_gate_fire, "到开抢时刻才允许提交(推荐开抢前用)。关掉 = 现在没入册就立刻抢(能提前锁定)");
            });
            ui.add_space(2.0);
            ui.label(egui::RichText::new("提示:已经提前预约入册的账号会被识别为“已入册”→ 到点做通道确认;万一到点被系统清掉,引擎第一时间抢回。token 与志愿会随配置存到 exe 同目录 cfg.json(请勿外传)。")
                .size(11.5).color(p.dim));
        });

        ui.add_space(8.0);
        // ---- 账号卡 ----
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("② 账号({})", d.sport_accounts.len())).size(15.0).strong().color(p.accent));
            if style::soft_button(ui, "恢复示例账号").clicked() {
                d.sport_accounts = default_sport_accounts();
                save_cfg(d, &d.cfg_path);
                d.log(Lvl::Info, "已恢复示例账号(请填入你自己的 token)。");
            }
            if style::soft_button(ui, "＋ 添加账号").clicked() {
                d.sport_accounts.push(model::SportAcc::new("新账号", "", &[], ""));
                save_cfg(d, &d.cfg_path);
            }
            if !d.sport_accounts.is_empty() && style::soft_button(ui, "移除最后一个").clicked() {
                d.sport_accounts.pop();
                save_cfg(d, &d.cfg_path);
            }
        });
        if d.sport_accounts.is_empty() {
            ui.label(egui::RichText::new("还没有账号:点“恢复默认 4 账号”或“添加账号”。").size(13.0).color(p.dim));
        }
        for i in 0..d.sport_accounts.len() {
            let snap = d.sport_accounts[i].clone();
            egui::Frame::none().fill(p.card).stroke(egui::Stroke::new(1.0, p.border))
                .rounding(egui::Rounding::same(10.0))
                .inner_margin(egui::Margin::same(12.0))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(&snap.name).size(14.0).strong());
                        if !snap.note.is_empty() {
                            ui.label(egui::RichText::new(&snap.note).size(11.5).color(p.accent));
                        }
                        let c = sport_st_color(&snap.st, &p);
                        style::chip(ui, snap.st_text(), c, style::tint(c, 20));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if style::accent_button(ui, "🔫 立即抢一次").clicked() {
                                app.send(Cmd::SportFireOne(i));
                            }
                        });
                    });
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label("token");
                        ui.add(egui::TextEdit::singleline(&mut d.sport_accounts[i].token)
                            .desired_width((ui.available_width() - 30.0).max(220.0))
                            .font(egui::TextStyle::Monospace)
                            .hint_text("粘贴 ACCESS_TOKEN"));
                    });
                    ui.horizontal(|ui| {
                        ui.label("志愿id");
                        ui.add(egui::TextEdit::singleline(&mut d.sport_accounts[i].ids_str)
                            .desired_width((ui.available_width() - 30.0).max(220.0))
                            .font(egui::TextStyle::Monospace)
                            .hint_text("如 1001,1002(逗号分隔,从上到下依次抢)"));
                    });
                    if !snap.sub.is_empty() {
                        ui.label(egui::RichText::new(&snap.sub).size(12.0).color(p.dim));
                    }
                });
            ui.add_space(5.0);
        }

        ui.add_space(8.0);
        // ---- 课程库 ----
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("③ 课程库({} 班)", d.sport_lib.len())).size(15.0).strong().color(p.accent));
            ui.add_space(8.0);
            ui.label("过滤");
            ui.add(egui::TextEdit::singleline(&mut d.sport_kw).desired_width(180.0)
                .hint_text("关键词,如 羽毛球/周一"));
        });
        if d.sport_lib.is_empty() {
            ui.label(egui::RichText::new("课程库为空 —— 点上方「载入课程库」拉取(需要任一有效 token)。").size(13.0).color(p.dim));
        } else {
            let kw = d.sport_kw.trim().to_lowercase();
            let rows: Vec<model::SportClass> = d.sport_lib.iter().filter(|c| {
                if kw.is_empty() { return true; }
                let hay = format!("{} {} {} {}", c.name.to_lowercase(), c.time.to_lowercase(),
                    c.teacher.to_lowercase(), c.loc.to_lowercase());
                hay.contains(&kw)
            }).take(400).cloned().collect();
            ui.add_space(4.0);
            egui::ScrollArea::vertical().id_salt("sport_lib").max_height(300.0).auto_shrink([false, false]).show(ui, |ui| {
                for (idx, c) in rows.iter().enumerate() {
                    let bg = if idx % 2 == 0 { p.card } else { p.card_alt };
                    egui::Frame::none().fill(bg).rounding(egui::Rounding::same(7.0))
                        .inner_margin(egui::Margin::symmetric(8.0, 3.0)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            if c.cap > 0 && c.used >= c.cap {
                                style::chip(ui, "已满", p.warn, style::tint(p.warn, 20));
                            } else if c.cap > 0 {
                                let cap_txt = format!("{}/{}", c.used, c.cap);
                                style::chip(ui, &cap_txt, p.ok, style::tint(p.ok, 20));
                            } else {
                                style::chip(ui, "—", p.dim, style::tint(p.dim, 18));
                            }
                            ui.label(egui::RichText::new(format!("#{}", c.id)).size(11.5).monospace().color(p.dim));
                            ui.label(egui::RichText::new(&c.name).size(13.0).strong());
                            ui.label(egui::RichText::new(&c.time).size(12.5).color(p.text));
                            ui.label(egui::RichText::new(&c.teacher).size(12.0).color(p.dim));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(egui::RichText::new(&c.loc).size(11.5).color(p.dim));
                                ui.add_space(8.0);
                                ui.label(egui::RichText::new(&c.sex).size(11.5).color(p.dim));
                            });
                        });
                    });
                    ui.add_space(2.0);
                }
            });
        }
    });
}

fn default_sport_accounts() -> Vec<model::SportAcc> {
    // 占位示例:token 与志愿班级请在「体育抢课」页填写(cfg.json 持久化,不会提交到仓库)。
    vec![
        model::SportAcc::new("示例账号1", "", &[], "在此填入你的 ACCESS_TOKEN 与志愿班级 id"),
        model::SportAcc::new("示例账号2", "", &[], "可删除或继续添加"),
    ]
}

// ---------------------------------------------------------------------------
// 日志页
// ---------------------------------------------------------------------------
fn page_log(ui: &mut egui::Ui, d: &mut AppData) {
    let p = style::palette_cur(ui);
    ui.horizontal(|ui| {
        ui.heading(egui::RichText::new("运行日志").size(20.0).strong());
        ui.add_space(8.0);
        if style::soft_button(ui, "清空").clicked() { d.logs.clear(); }
    });
    ui.add_space(8.0);
    let dark = ui.ctx().style().visuals.dark_mode;
    let h = (ui.available_height() * 0.62).max(160.0);
    egui::Frame::none().fill(p.card).stroke(egui::Stroke::new(1.0, p.border))
        .rounding(egui::Rounding::same(12.0)).inner_margin(egui::Margin::same(6.0))
        .show(ui, |ui| {
        ui.set_height(h);
        egui::ScrollArea::vertical().id_salt("logs").stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
            if d.logs.is_empty() {
                ui.label(egui::RichText::new("暂无日志。").size(13.0).color(p.dim));
            }
            for l in d.logs.iter() {
                ui.horizontal(|ui| {
                    ui.add_sized([52.0, 16.0], egui::Label::new(egui::RichText::new(&l.t).size(11.0).monospace().color(p.dim)));
                    let c = style::lvl_color(l.lvl, dark);
                    ui.add_sized([26.0, 16.0], egui::Label::new(egui::RichText::new(l.lvl.tag()).size(11.0).monospace().strong().color(c)));
                    ui.label(egui::RichText::new(&l.msg).size(12.5));
                });
            }
        });
    });
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("最近接口原始返回(调试):").size(12.0).color(p.dim));
        if style::soft_button(ui, "复制原始内容").clicked() {
            ui.ctx().output_mut(|o| o.copied_text = d.last_raw.clone());
        }
        if style::soft_button(ui, "清空").clicked() { d.last_raw.clear(); }
    });
    if !d.last_raw.is_empty() {
        egui::ScrollArea::vertical().id_salt("raw").max_height(140.0).auto_shrink([false, false]).show(ui, |ui| {
            ui.add(egui::Label::new(egui::RichText::new(&d.last_raw).monospace().size(11.5)).wrap().selectable(true));
        });
    }
}

// ---------------------------------------------------------------------------
// 高级
// ---------------------------------------------------------------------------
fn page_advanced(ui: &mut egui::Ui, d: &mut AppData) {
    let p = style::palette_cur(ui);
    ui.heading(egui::RichText::new("高级 · 接口模板").size(20.0).strong());
    ui.add_space(2.0);
    ui.label(egui::RichText::new("默认按 URP/eams 同系标准接口预填(提交/列表 action 已内建,含自研容错 JS 解析)。学校改版导致拉不到列表/提交失败时,用下面“cURL 导入”按真实请求修正。")
        .size(12.5).color(p.dim));
    ui.add_space(10.0);

    style::card(ui, "请求模板", |ui| {
        egui::Grid::new("adv").num_columns(2).spacing([16.0, 8.0]).striped(true).show(ui, |ui| {
            ui.label("列表 action");
            ui.add(egui::TextEdit::singleline(&mut d.data_path).desired_width(480.0).font(egui::TextStyle::Monospace));
            ui.end_row();
            ui.label("余量附加参数");
            ui.add(egui::TextEdit::singleline(&mut d.count_extra).desired_width(480.0).hint_text("以 & 开头,如 &semesterId=…").font(egui::TextStyle::Monospace));
            ui.end_row();
            ui.label("提交 action");
            ui.add(egui::TextEdit::singleline(&mut d.submit_path).desired_width(480.0).font(egui::TextStyle::Monospace));
            ui.end_row();
            ui.label("提交 query 模板");
            ui.add(egui::TextEdit::singleline(&mut d.submit_query).desired_width(480.0).hint_text("profileId={pid}").font(egui::TextStyle::Monospace));
            ui.end_row();
            ui.label("提交 body 模板");
            ui.add(egui::TextEdit::singleline(&mut d.submit_body).desired_width(480.0).hint_text("optype=true&operator0={id}:true:0").font(egui::TextStyle::Monospace));
            ui.end_row();
        });
        ui.add_space(4.0);
        ui.label(egui::RichText::new("{pid}=批次编号,{id}=教学班 id。改完点“保存设置”。").size(11.5).color(p.dim));
    });

    ui.add_space(10.0);
    style::card(ui, "cURL 一键导入(提交请求)", |ui| {
        ui.label(egui::RichText::new("浏览器选课页点一次“选课”→ F12 → Network 找到 POST → 右键 Copy as cURL → 整段粘贴:").size(12.5).color(p.dim));
        ui.add_space(6.0);
        ui.add(egui::TextEdit::multiline(&mut d.curl_input).desired_rows(4).desired_width(f32::INFINITY)
            .hint_text("curl 'http://…/stdElectCourse!…' -H '…' --data-raw '…'").font(egui::TextStyle::Monospace));
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if style::accent_button(ui, "解析并应用").clicked() {
                let n = import_curl(d);
                if n > 0 {
                    d.log(Lvl::Ok, format!("cURL 解析成功,已套用 {} 项(模板/编号/Cookie)。", n));
                } else {
                    d.log(Lvl::Err, "cURL 解析失败:请确认复制的是 DevTools 的完整 cURL(以 curl 开头)。");
                }
            }
            if style::soft_button(ui, "保存设置").clicked() { save_cfg(d, &d.cfg_path); d.log(Lvl::Info, "设置已保存。"); }
        });
    });

    ui.add_space(10.0);
    style::card(ui, "配置与隐私", |ui| {
        ui.horizontal(|ui| {
            ui.label("配置文件:");
            ui.label(egui::RichText::new(&d.cfg_path).size(12.0).monospace().color(p.dim));
        });
        ui.horizontal(|ui| {
            if style::soft_button(ui, "立即保存").clicked() { save_cfg(d, &d.cfg_path); d.log(Lvl::Info, "配置已保存。"); }
            if style::soft_button(ui, "重新载入").clicked() { let cp = d.cfg_path.clone(); load_cfg(d, &cp); d.log(Lvl::Info, "配置已从磁盘重载。"); }
            if style::soft_button(ui, "清除 Cookie").clicked() {
                d.cookie.clear(); d.session_msg.clear();
                save_cfg(d, &d.cfg_path);
                d.log(Lvl::Info, "Cookie 已清除并保存。");
            }
            ui.label(egui::RichText::new("Cookie 只存于 exe 同目录 cfg.json,不上传;密码永不落盘。").size(11.5).color(p.dim));
        });
    });
    ui.add_space(10.0);
    ui.label(egui::RichText::new("说明:本工具等价于自动模拟你自己在浏览器里点“选课”,不绕过任何鉴权(仍需本人统一身份认证)。请只在教务规定时间内选课,保持合理请求间隔,勿给服务器造成过大压力;使用后果由使用者自行承担。")
        .size(11.5).color(p.dim));
}

// ---------------------------------------------------------------------------
// cURL 解析
// ---------------------------------------------------------------------------
fn import_curl(d: &mut AppData) -> usize {
    let s = d.curl_input.trim();
    if !s.starts_with("curl") { return 0; }
    let mut n = 0usize;

    let take_quoted = |rest: &str| -> Option<(String, usize)> {
        let c = rest.chars().next()?;
        if c != '\'' && c != '"' { return None; }
        let mut end = None;
        for (idx, ch) in rest.char_indices().skip(1) {
            if ch == c { end = Some(idx); break; }
        }
        let e = end?;
        Some((rest[1..e].to_string(), e + 1))
    };

    // 1) url
    let rest0 = &s["curl".len()..];
    let trimmed = rest0.trim_start();
    let (url, after_url) = match take_quoted(trimmed) {
        Some(x) => x,
        None => return 0,
    };
    let rest = &trimmed[after_url..];

    // 2) 顺序解析 -H / --data-raw / -d / --data
    let mut cookie = String::new();
    let mut body = String::new();
    let mut query_tpl: Option<String> = None;
    let mut rel_action: Option<String> = None;

    let mut rest = rest;
    loop {
        let rest_t = rest.trim_start();
        if rest_t.is_empty() { break; }
        if let Some(r2) = rest_t.strip_prefix("-H ").or_else(|| rest_t.strip_prefix("--header ")) {
            if let Some((val, used)) = take_quoted(r2.trim_start()) {
                if let Some((k, v)) = val.split_once(':') {
                    if k.trim().eq_ignore_ascii_case("cookie") {
                        cookie = v.trim().to_string();
                        n += 1;
                    }
                }
                rest = &r2[used..];
            } else { break; }
        } else if let Some(r2) = rest_t.strip_prefix("--data-raw ").or_else(|| rest_t.strip_prefix("--data ")).or_else(|| rest_t.strip_prefix("-d ")) {
            if let Some((val, used)) = take_quoted(r2.trim_start()) {
                body = val;
                rest = &r2[used..];
            } else { break; }
        } else {
            // 跳过下一个空白或引号
            match rest_t.find(' ') {
                Some(sp) => rest = &rest_t[sp..],
                None => break,
            }
        }
    }

    // cookie
    if !cookie.is_empty() { d.cookie = cookie; }

    // 3) 解析 url
    if let Some((path, query)) = url.split_once('?') {
        if let Some(ep) = path.find("/eams/") {
            let rel = &path[ep + 6..];
            if !rel.is_empty() && !rel.contains("login") {
                rel_action = Some(rel.to_string());
            }
            // profile id
            for key in ["electionProfile.id=", "profileId="] {
                if let Some(pos) = query.find(key) {
                    let v: String = query[pos + key.len()..].chars().take_while(|c| c.is_ascii_digit()).collect();
                    if !v.is_empty() { d.profile = v; n += 1; }
                }
            }
            // 把 query 模板化
            let tpl: Vec<String> = query.split('&').filter(|kv| !kv.is_empty()).map(|kv| {
                if let Some((k, _)) = kv.split_once('=') {
                    if k == "profileId" || k == "electionProfile.id" {
                        return format!("{}={{{{pid}}}}", k);
                    }
                }
                kv.to_string()
            }).collect();
            if !tpl.is_empty() {
                query_tpl = Some(tpl.join("&"));
            }
        }
    }
    if let Some(q) = query_tpl { d.submit_query = q; n += 1; }
    if let Some(a) = rel_action { d.submit_path = a; n += 1; }

    // body -> 模板化
    if !body.is_empty() {
        let mut nb = body.clone();
        while let Some(pos) = nb.find("operator0=") {
            let tail = &nb[pos + "operator0=".len()..];
            let digits: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
            if digits.is_empty() { break; }
            nb = nb.replacen(&format!("operator0={}", digits), "operator0={id}", 1);
        }
        d.submit_body = nb;
        n += 1;
    }
    n
}
