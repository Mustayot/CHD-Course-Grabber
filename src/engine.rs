//! 引擎线程:统一处理 UI 命令,并驱动“蹲守/抢课”主循环。
use crate::model::{AppData, CourseRow, Lvl, Strategy, TState, Target};
use crate::net::{self, Api};
use crate::sport;
use eframe::egui;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

pub enum Cmd {
    TestSession,
    FetchElections,
    FetchCourses,
    ProbeLessons,           // 探测 defaultPage, 解析 lesson 列表, 落盘 HTML
    AutoLogin,
    QuickGrab(Strategy, String), // 关键词自动抢:目标策略 + 关键词
    Start,
    Stop,
    // ---- 体育(stuh5) ----
    SportLoad,              // 用第一个账号的 token 载入体育课程库
    SportVerify,            // 验证全部体育账号 token
    SportArm,               // 启动体育引擎(盯防/到点抢/自动确认)
    SportStop,              // 停止体育引擎
    SportFireOne(usize),    // 手动对某个账号立即按志愿抢一次
}

pub fn spawn(ctx: egui::Context, data: Arc<Mutex<AppData>>) -> (mpsc::Sender<Cmd>, Arc<AtomicBool>) {
    let (tx, rx) = mpsc::channel::<Cmd>();
    let run = Arc::new(AtomicBool::new(false));
    let run2 = run.clone();
    std::thread::spawn(move || loop {
        match rx.recv_timeout(Duration::from_millis(120)) {
            Ok(cmd) => handle(cmd, &ctx, &data, &run2),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    });
    (tx, run)
}

fn handle(cmd: Cmd, ctx: &egui::Context, data: &Arc<Mutex<AppData>>, run: &AtomicBool) {
    let (base, pid, cookie) = {
        let g = data.lock().unwrap();
        (g.base.clone(), g.profile.clone(), g.cookie.clone())
    };
    let mut api = Api::new();
    if !cookie.is_empty() { api.set_cookie_str(&cookie); }

    match cmd {
        Cmd::TestSession => {
            let msg = test_session(&mut api, &base, &pid);
            log_msg(data, if msg.starts_with('✓') { Lvl::Ok } else { Lvl::Err }, &msg);
            let mut g = data.lock().unwrap();
            g.session_msg = msg;
            ctx.request_repaint();
        }
        Cmd::FetchElections => {
            let ok = fetch_elections(&mut api, &base, data);
            log_msg(data, if ok { Lvl::Ok } else { Lvl::Err },
                if ok { "已刷新可选批次(可在顶部下拉里切换)" } else { "刷新批次失败,请先检查会话" });
            ctx.request_repaint();
        }
        Cmd::FetchCourses => {
            fetch_courses(&mut api, &base, &pid, data);
            ctx.request_repaint();
        }
        Cmd::ProbeLessons => {
            let msg = probe_lessons(&mut api, &base, &pid, data);
            log_msg(data, Lvl::Info, &msg);
            ctx.request_repaint();
        }
        Cmd::AutoLogin => {
            let (u, p) = {
                let g = data.lock().unwrap();
                (g.account.clone(), g.password.clone())
            };
            match net::try_login(&mut api, &base, &u, &p) {
                Ok(cs) => {
                    let mut g = data.lock().unwrap();
                    g.cookie = cs.clone();
                    g.password.clear();
                    g.session_msg = "已登录 ✓(自动登录成功)".into();
                    g.log(Lvl::Ok, "自动登录成功,会话 Cookie 已写入(仅存本机 cfg.json,不联网上传)");
                    ctx.request_repaint();
                }
                Err(e) => {
                    let msg = format!("自动登录失败:{}", e);
                    let mut g = data.lock().unwrap();
                    g.password.clear();
                    g.session_msg = msg.clone();
                    g.log(Lvl::Err, msg);
                    ctx.request_repaint();
                }
            }
        }
        Cmd::QuickGrab(strat, kw) => quick_grab(ctx, data, run, strat, &kw),
        Cmd::Start => start_engine(ctx, data, run),
        Cmd::Stop => {
            run.store(false, Ordering::SeqCst);
            log_msg(data, Lvl::Info, "收到停止请求,正在收尾…");
            ctx.request_repaint();
        }
        Cmd::SportLoad => cmd_sport_load(ctx, data),
        Cmd::SportVerify => cmd_sport_verify(ctx, data),
        Cmd::SportArm => cmd_sport_arm(ctx, data),
        Cmd::SportStop => cmd_sport_stop(ctx, data),
        Cmd::SportFireOne(ai) => {
            let msg = sport::manual_fill(data, ai);
            log_msg(data, Lvl::Info, msg);
            ctx.request_repaint();
        }
    }
}

fn cmd_sport_load(ctx: &egui::Context, data: &Arc<Mutex<AppData>>) {
    let token = {
        let g = data.lock().unwrap();
        g.sport_accounts.iter().find(|a| !a.token.trim().is_empty()).map(|a| a.token.clone())
    };
    let Some(tk) = token else {
        log_msg(data, Lvl::Err, "请先在「体育抢课」页填写至少一个账号的 token。");
        return;
    };
    match sport::load_library(&tk) {
        Ok(list) => {
            let n = list.len();
            {
                let mut g = data.lock().unwrap();
                g.sport_lib = list;
            }
            log_msg(data, Lvl::Ok, &format!("体育课程库已载入 {} 个教学班(可到页面下方检索/核对)。", n));
        }
        Err(e) => {
            log_msg(data, Lvl::Err, &format!("载入体育课程库失败:{}", e));
        }
    }
    ctx.request_repaint();
}

fn cmd_sport_verify(ctx: &egui::Context, data: &Arc<Mutex<AppData>>) {
    let accs: Vec<(usize, String, String)> = {
        let g = data.lock().unwrap();
        g.sport_accounts.iter().enumerate()
            .map(|(i, a)| (i, a.name.clone(), a.token.clone()))
            .collect()
    };
    let mut ok_n = 0usize;
    for (ai, name, token) in accs {
        if token.trim().is_empty() { continue; }
        match sport::check_scores(&token) {
            Ok(recs) => {
                ok_n += 1;
                let note = if recs.is_empty() {
                    "token 有效 · 未入册(启动引擎即抢)".to_string()
                } else {
                    format!("✅ 已入册:{}", recs[0])
                };
                {
                    let mut g = data.lock().unwrap();
                    if ai < g.sport_accounts.len() {
                        let a = &mut g.sport_accounts[ai];
                        a.st = if recs.is_empty() { "armed".into() } else { "ok".into() };
                        a.sub = note;
                    }
                }
                log_msg(data, Lvl::Ok, &format!("{}:token 有效({})", name, if recs.is_empty() { "未入册" } else { "已入册" }));
            }
            Err(e) if e.starts_with("401") => {
                {
                    let mut g = data.lock().unwrap();
                    if ai < g.sport_accounts.len() {
                        g.sport_accounts[ai].st = "err".into();
                        g.sport_accounts[ai].sub = "token 失效(401)".into();
                    }
                }
                log_msg(data, Lvl::Err, &format!("{}:token 失效(401),请换新。", name));
            }
            Err(e) => {
                log_msg(data, Lvl::Warn, &format!("{}:验证异常:{}", name, e));
            }
        }
    }
    if ok_n > 0 {
        log_msg(data, Lvl::Ok, &format!("体育账号验证完成(有效 {} 个)。", ok_n));
    }
    ctx.request_repaint();
}

fn cmd_sport_arm(ctx: &egui::Context, data: &Arc<Mutex<AppData>>) {
    let busy = data.lock().unwrap().sport_active;
    if busy {
        log_msg(data, Lvl::Warn, "体育引擎已在运行,先停止再重新启动。");
        return;
    }
    let has_token = {
        let g = data.lock().unwrap();
        g.sport_accounts.iter().any(|a| !a.token.trim().is_empty())
    };
    if !has_token {
        log_msg(data, Lvl::Err, "请先填写至少一个账号的 token。");
        return;
    }
    sport::run_engine(ctx.clone(), data.clone());
    ctx.request_repaint();
}

fn cmd_sport_stop(ctx: &egui::Context, data: &Arc<Mutex<AppData>>) {
    {
        let mut g = data.lock().unwrap();
        g.sport_active = false;
        g.sport_msg = "已停止".into();
        g.log(Lvl::Info, "体育引擎已请求停止。");
        for a in g.sport_accounts.iter_mut() {
            if a.st == "run" || a.st == "armed" { a.st = "idle".into(); }
        }
    }
    ctx.request_repaint();
}

fn log_msg(data: &Arc<Mutex<AppData>>, lvl: Lvl, msg: impl Into<String>) {
    data.lock().unwrap().log(lvl, msg.into());
}

/// 统一启动引擎(阻塞执行 run_loop,直至停止/全部抢中)
fn start_engine(ctx: &egui::Context, data: &Arc<Mutex<AppData>>, run: &AtomicBool) {
    if run.load(Ordering::SeqCst) { return; }
    let n = {
        let mut g = data.lock().unwrap();
        g.targets.iter_mut().for_each(|t| { t.state = TState::Idle; t.tries = 0; t.last.clear(); t.ok = false; });
        let strat_lbl = g.strategy.label();
        let (rms, sms) = (g.refresh_ms, g.submit_ms);
        g.running = true;
        g.rounds = 0;
        g.engine_label = format!("▶ 运行中 · 模式:{}", strat_lbl);
        g.started_at = Some(chrono::Local::now().format("%H:%M:%S").to_string());
        g.log(Lvl::Info, format!("▶ 启动,模式:{} / 刷新 {}ms / 提交 {}ms", strat_lbl, rms, sms));
        g.targets.iter().filter(|t| t.enabled).count()
    };
    if n == 0 {
        log_msg(data, Lvl::Err, "没有启用的抢课目标:请先在“课程检索”检索并按 ＋ 加入目标,或用“快捷自动抢”。");
        let mut g = data.lock().unwrap();
        g.running = false;
        g.engine_label = "空闲".into();
        return;
    }
    run.store(true, Ordering::SeqCst);
    run_loop(ctx, data, run);
    run.store(false, Ordering::SeqCst);
    let mut g = data.lock().unwrap();
    g.running = false;
    g.engine_label = "已停止".into();
    let p = g.cfg_path.clone();
    g.log(Lvl::Info, "⏹ 引擎已停止。");
    drop(g);
    save_app_cfg(data, &p);
    ctx.request_repaint();
}

fn save_app_cfg(data: &Arc<Mutex<AppData>>, path: &str) {
    let g = data.lock().unwrap();
    crate::save_cfg(&g, path);
}

/// 关键词自动抢:确保课程已拉取 -> 按关键词过滤 -> 去重加入目标 -> 套用策略并启动。
fn quick_grab(ctx: &egui::Context, data: &Arc<Mutex<AppData>>, run: &AtomicBool, strat: Strategy, kw: &str) {
    if run.load(Ordering::SeqCst) {
        log_msg(data, Lvl::Err, "引擎正在运行,请先停止再使用“快捷自动抢”。");
        return;
    }
    let kw = kw.trim();
    if kw.is_empty() {
        log_msg(data, Lvl::Warn, "快捷自动抢:请先输入课程关键词(如“大学物理”或课程编号)。");
        return;
    }
    let (base, pid, cookie, cfg_path) = {
        let g = data.lock().unwrap();
        (g.base.clone(), g.profile.clone(), g.cookie.clone(), g.cfg_path.clone())
    };
    if cookie.trim().is_empty() {
        log_msg(data, Lvl::Err, "还没有会话 Cookie:请先用“内置浏览器登录”或粘贴 Cookie 完成登录。");
        return;
    }
    // 1) 课程列表为空就先拉
    let empty = data.lock().unwrap().courses.is_empty();
    if empty {
        log_msg(data, Lvl::Info, "课程列表为空,先自动拉取本批次课程…");
        let mut api = Api::new();
        api.set_cookie_str(&cookie);
        fetch_courses(&mut api, &base, &pid, data);
    }
    // 2) 关键词过滤(课程名/编号/教师,空格分词 AND)
    let kws: Vec<String> = kw.split(|c: char| c == ' ' || c == '、' || c == ',' || c == '，')
        .map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect();
    let matched: Vec<CourseRow> = {
        let g = data.lock().unwrap();
        g.courses.iter().filter(|c| {
            let hay = format!("{} {} {}", c.no.to_lowercase(), c.name.to_lowercase(), c.teachers.to_lowercase());
            kws.iter().all(|k| hay.contains(k.as_str()))
        }).cloned().collect()
    };
    if matched.is_empty() {
        log_msg(data, Lvl::Warn, &format!("关键词“{}”没有命中任何教学班(已拉取 {} 门)。请换个关键词或先核对批次。", kw, data.lock().unwrap().courses.len()));
        return;
    }
    // 3) 去重加入目标
    let (added, dup) = {
        let mut g = data.lock().unwrap();
        let mut added = 0usize;
        let mut dup = 0usize;
        for c in &matched {
            if g.targets.iter().any(|t| t.id == c.id) { dup += 1; continue; }
            g.targets.push(Target::new(c.id, &c.no, &c.name));
            added += 1;
        }
        (added, dup)
    };
    if added == 0 {
        log_msg(data, Lvl::Warn, &format!("“{}”命中的教学班都已存在于目标列表(新增 0)。", kw));
    } else {
        log_msg(data, Lvl::Ok, &format!("关键词“{}”命中 {} 个教学班,已加入目标 {} 个(跳过重复 {} 个)。", kw, matched.len(), added, dup));
    }
    // 4) 套用策略并启动
    {
        let mut g = data.lock().unwrap();
        g.strategy = strat;
    }
    save_app_cfg(data, &cfg_path);
    start_engine(ctx, data, run);
}

fn test_session(api: &mut Api, base: &str, pid: &str) -> String {
    match api.get(&net::url_default(base, pid)) {
        Err(e) => format!("✗ 请求失败:{}", e),
        Ok(r) => {
            let login = Api::is_login_page(&r.final_url, &r.text);
            if login || r.status == 401 {
                format!("✗ 会话无效 —— 被跳转到登录页({})。请浏览器登录后复制 Cookie。", host_of(&r.final_url))
            } else if r.status == 200 {
                let hint = if r.text.contains("选课") { "页面含选课内容" } else { "页面已打开" };
                format!("✓ 会话有效:HTTP 200 @ {} ,返回 {} 字节({})。可以开始拉取课程。", host_of(&r.final_url), r.text.len(), hint)
            } else {
                format!("✗ HTTP {} @ {}", r.status, host_of(&r.final_url))
            }
        }
    }
}

fn host_of(u: &str) -> String {
    u.replace("https://", "").replace("http://", "").split(['/', '?']).next().unwrap_or(u).to_string()
}

fn fetch_elections(api: &mut Api, base: &str, data: &Arc<Mutex<AppData>>) -> bool {
    // 先试选课首页 stdElectCourse.action(列出当前学生的所有可进轮次),再退回 innerIndex。
    for url in [net::url_elect_home(base), net::url_inner(base)] {
        match api.get(&url) {
            Err(e) => { log_msg(data, Lvl::Err, &format!("获取批次失败:{}", e)); return false; }
            Ok(r) => {
                if Api::is_login_page(&r.final_url, &r.text) {
                    log_msg(data, Lvl::Err, "会话无效,先登录/导入 Cookie 再刷新批次。");
                    return false;
                }
                let mut list: Vec<(String, String)> = Vec::new();
                let mut idx = 0usize;
                while let Some(off) = r.text[idx..].find("electionProfile.id=") {
                    let i = idx + off;
                    let seg = &r.text[i + "electionProfile.id=".len()..];
                    let mut id = String::new();
                    for c in seg.chars() { if c.is_ascii_digit() { id.push(c); } else { break; } }
                    if !id.is_empty() {
                        // 取该链接前面最近的可见文字作为轮次名(倒查 400 字符内的标签外文本)
                        let back = &r.text[..i];
                        let name = {
                            let win = back.len().saturating_sub(500);
                            let w = &back[win..];
                            let mut texts: Vec<String> = Vec::new();
                            let mut cur = String::new();
                            let mut in_tag = false;
                            for c in w.chars() {
                                if c == '<' { if !cur.trim().is_empty() { texts.push(cur.trim().to_string()); } cur.clear(); in_tag = true; }
                                else if c == '>' { in_tag = false; }
                                else if !in_tag { cur.push(c); }
                            }
                            if !cur.trim().is_empty() { texts.push(cur.trim().to_string()); }
                            texts.into_iter().rev().find(|t| t.chars().count() >= 2 && !t.contains("进入选课"))
                                .unwrap_or_else(|| "批次".into())
                        };
                        if !list.iter().any(|(_, x)| *x == id) { list.push((name, id)); }
                    }
                    idx = i + 20;
                }
                if !list.is_empty() {
                    let mut g = data.lock().unwrap();
                    g.elections = list.clone();
                    drop(g);
                    log_msg(data, Lvl::Ok, &format!("发现 {} 个可选轮次(取当前有效轮次)。", list.len()));
                    // 自动切换 profile 到第一个
                    if let Some((_, first)) = list.first() {
                        let mut g = data.lock().unwrap();
                        g.profile = first.clone();
                    }
                    return true;
                }
                // 继续试下一个 url
            }
        }
    }
    log_msg(data, Lvl::Warn, "未在该页面找到可选轮次(可能当前无开放窗口)。可手工在顶部填 electionProfile.id,或点“在浏览器中打开选课页”查看。");
    true
}

fn fetch_courses(api: &mut Api, base: &str, pid: &str, data: &Arc<Mutex<AppData>>) {
    let cfg_dir = { data.lock().unwrap().cfg_path.clone() };
    let dump_dir = std::path::Path::new(&cfg_dir)
        .parent().unwrap_or(std::path::Path::new("."))
        .to_path_buf();

    // === 阶段 1: GET defaultPage —— 在服务端建立"当前轮次"上下文 + 提取 semesterId ===
    let url_def = net::url_default(base, pid);
    let r_def = match api.get(&url_def) {
        Err(e) => { log_msg(data, Lvl::Err, &format!("拉取 defaultPage 失败:{}", e)); return; }
        Ok(r) => r,
    };
    if Api::is_login_page(&r_def.final_url, &r_def.text) {
        log_msg(data, Lvl::Err, "会话无效:defaultPage 被踢到登录页,请重新登录。");
        return;
    }
    // 轮次 id 失效 / 不在选课时间: eams 会返回 "请求参数非法" 小页面
    if r_def.text.contains("请求参数非法") || r_def.text.len() < 100 {
        log_msg(data, Lvl::Err, &format!(
            "选课页返回“请求参数非法”({} 字节)——当前轮次 #{} 不可用(已到截止时间/未开放/非你可选)。\n请点“自动发现轮次”拿当前有效编号,或在浏览器打开选课首页确认。",
            r_def.text.len(), pid));
        return;
    }
    // 无条件落盘供诊断
    let dump_html = dump_dir.join("defaultpage_debug.html");
    let _ = std::fs::write(&dump_html, &r_def.text);
    if let Some(sem) = net::extract_semester_id(&r_def.text) {
        let changed = {
            let mut g = data.lock().unwrap();
            let old = g.semester_id.clone();
            if old != sem { g.semester_id = sem.clone(); true } else { false }
        };
        if changed { log_msg(data, Lvl::Ok, &format!("已从选课页提取 semesterId = {}", sem)); }
    }

    // === 阶段 2: data.action 一把梭返回该轮次全部教学班(已实测,无需按 lesson 分组) ===
    let url_data = format!("{}/eams/stdElectCourse!data.action?profileId={}", base.trim_end_matches('/'), pid);
    let resp_data = match api.get(&url_data) {
        Err(e) => { log_msg(data, Lvl::Err, &format!("拉取课程失败:{}", e)); return; }
        Ok(r) => r,
    };
    if Api::is_login_page(&resp_data.final_url, &resp_data.text) {
        log_msg(data, Lvl::Err, "课程接口被踢到登录页,会话可能已失效,请重新登录。");
        return;
    }
    let mut rows = net::parse_course_list(&resp_data.text);
    log_msg(data, Lvl::Info, &format!("课程接口返回 {} 字节 → {} 个教学班", resp_data.text.len(), rows.len()));
    if rows.is_empty() {
        let dump = dump_dir.join("fetch_debug_response.txt");
        let _ = std::fs::write(&dump, &resp_data.text);
        log_msg(data, Lvl::Err, &format!(
            "该轮次返回 0 个教学班:可能还没放课,或轮次已关闭。原始响应已落盘:\n{} ({} 字节)。轮次状态也可用“🔍 仅探测 Lesson”复核。",
            dump.display(), resp_data.text.len()));
        return;
    }

    // === 阶段 3: 余量 queryStdCount?projectId=1&semesterId=xxx ===
    let (sem, extra) = {
        let g = data.lock().unwrap();
        (g.semester_id.clone(), g.count_extra.clone())
    };
    match api.get(&net::url_counts_v2(base, &sem, &extra)) {
        Ok(r) if !Api::is_login_page(&r.final_url, &r.text) => {
            let m = net::parse_counts(&r.text);
            if m.is_empty() {
                log_msg(data, Lvl::Warn, "余量接口未解析出数据(semesterId 可能需要更新,见会话页/高级页)。");
            } else {
                log_msg(data, Lvl::Ok, &format!("余量同步 {} 个教学班。", m.len()));
                for row in rows.iter_mut() {
                    if let Some((sc, lc)) = m.get(&row.id.to_string()) { row.sc = Some(*sc); row.lc = Some(*lc); }
                }
            }
        }
        Ok(_) => log_msg(data, Lvl::Warn, "余量接口被踢登录页,已跳过。"),
        Err(e) => log_msg(data, Lvl::Warn, &format!("余量拉取失败:{}", e)),
    }

    let n = rows.len();
    data.lock().unwrap().courses = rows;
    log_msg(data, Lvl::Ok, &format!("就绪:{} 个教学班可在下方检索/加入目标。", n));
}

/// 探测 defaultPage 页面:落盘 HTML + 解析 lesson + 报告。
fn probe_lessons(api: &mut Api, base: &str, pid: &str, data: &Arc<Mutex<AppData>>) -> String {
    let cfg_dir = { data.lock().unwrap().cfg_path.clone() };
    let dump = std::path::Path::new(&cfg_dir)
        .parent().unwrap_or(std::path::Path::new("."))
        .join("defaultpage_debug.html");
    let url_def = net::url_default(base, pid);
    match api.get(&url_def) {
        Err(e) => format!("探测失败:{}", e),
        Ok(r) => {
            let _ = std::fs::write(&dump, &r.text);
            if Api::is_login_page(&r.final_url, &r.text) {
                return format!("被踢到登录页,先登录。HTML 已落盘:{}", dump.display());
            }
            let lessons = net::parse_lesson_list(&r.text);
            if lessons.is_empty() {
                format!("已落盘 defaultPage HTML ({} 字节) 到 {}\n但未解析出 lesson (HTML 结构可能非标准,请把该文件发给开发者分析)。",
                        r.text.len(), dump.display())
            } else {
                let list = lessons.iter().take(50)
                    .map(|(id, n)| format!("  - lesson.id={}  {}", id, n))
                    .collect::<Vec<_>>().join("\n");
                let more = if lessons.len() > 50 { format!("\n  …还有 {} 个", lessons.len() - 50) } else { String::new() };
                format!("发现 {} 个 lesson:\n{}{}\nHTML 已落盘:{}", lessons.len(), list, more, dump.display())
            }
        }
    }
}

fn trunc(s: &str, n: usize) -> String {
    let t: String = s.chars().filter(|c| !c.is_control()).collect();
    if t.chars().count() > n { t.chars().take(n).collect() } else { t }
}

// ---------------------------------------------------------------------------
// 主循环
// ---------------------------------------------------------------------------
fn run_loop(ctx: &egui::Context, data: &Arc<Mutex<AppData>>, run: &AtomicBool) {
    let (base, pid, cookie) = {
        let g = data.lock().unwrap();
        (g.base.clone(), g.profile.clone(), g.cookie.clone())
    };
    let mut api = Api::new();
    if !cookie.is_empty() { api.set_cookie_str(&cookie); }

    let mut full_since: HashMap<u64, u64> = HashMap::new(); // id -> 变满的轮次
    let mut was_free: HashMap<String, Option<bool>> = HashMap::new();
    let mut last_submit = Instant::now();
    let mut cycle_no: u64 = 0;
    let full_skip_cycles: u64 = 6;

    while run.load(Ordering::SeqCst) {
        let cyc = Instant::now();
        cycle_no += 1;

        // ---- 1) 每轮刷新余量(queryStdCount 实测参数 projectId+semesterId) ----
        let (sem, extra) = {
            let g = data.lock().unwrap();
            (g.semester_id.clone(), g.count_extra.clone())
        };
        let count_res = api.get(&net::url_counts_v2(&base, &sem, &extra));
        match count_res {
            Ok(r) if !Api::is_login_page(&r.final_url, &r.text) => {
                let m = net::parse_counts(&r.text);
                if m.is_empty() {
                    if cycle_no == 1 { log_msg(data, Lvl::Warn, "余量接口无数据(直抢模式不受影响,照常提交)。"); }
                } else {
                    for (id, (sc, lc)) in &m {
                        let free = sc < lc;
                        let prev = was_free.insert(id.clone(), Some(free)).flatten();
                        if prev == Some(false) && free {
                            if let Some(name) = target_name(data, id) {
                                log_msg(data, Lvl::Succ, format!("🎯 目标“{}”出现空位!已选 {}/容量 {} → 立刻开抢。", name, sc, lc));
                            }
                        }
                    }
                }
                // 回写 sc/lc
                let mut g = data.lock().unwrap();
                for (id, (sc, lc)) in &m {
                    for t in g.targets.iter_mut() {
                        if t.id.to_string() == *id { t.sc = Some(*sc); t.lc = Some(*lc); }
                    }
                    for c in g.courses.iter_mut() {
                        if c.id.to_string() == *id { c.sc = Some(*sc); c.lc = Some(*lc); }
                    }
                }
                g.rounds = cycle_no;
                g.engine_label = format!("▶ 运行中 · 第 {} 轮 · 刷新 {}ms / 提交 {}ms", cycle_no, g.refresh_ms, g.submit_ms);
            }
            Ok(_) => {
                log_msg(data, Lvl::Err, "会话已失效(被踢到登录页),自动停止。请重新登录并更新 Cookie 后再开始。");
                break;
            }
            Err(e) => {
                if cycle_no % 12 == 1 { log_msg(data, Lvl::Warn, &format!("余量刷新网络异常:{}", e)); }
            }
        }
        ctx.request_repaint();

        // ---- 2) 提交阶段 ----
        let strat = data.lock().unwrap().strategy;
        if strat != Strategy::Watch {
            let todo: Vec<Target> = {
                let g = data.lock().unwrap();
                g.targets.iter().filter(|t| t.enabled && !t.ok).cloned().collect()
            };
            let submit_ms = data.lock().unwrap().submit_ms.max(60);
            for t in &todo {
                if !run.load(Ordering::SeqCst) { break; }
                // 满员退避:变满后歇几轮,避免死磕服务器
                let since_full = cycle_no.saturating_sub(*full_since.get(&t.id).unwrap_or(&0));
                if t.state == TState::Full && since_full < full_skip_cycles { continue; }
                if strat == Strategy::Free && !t.free() { continue; }

                // 全局提交节流
                let gap = last_submit.elapsed().as_millis() as u64;
                if gap < submit_ms { wait_ms(run, submit_ms - gap); }
                if !run.load(Ordering::SeqCst) { break; }

                let out = do_submit(&mut api, &base, &pid, t, data, &mut last_submit);
                match out {
                    Out::Ok => {
                        // 同课程编号抢到一门后,自动停用其余教学班,防重复/冲突
                        if data.lock().unwrap().one_per_no {
                            let no = t.no.clone();
                            if !no.is_empty() {
                                let n_dis = disable_siblings(data, &no, t.id);
                                if n_dis > 0 {
                                    log_msg(data, Lvl::Ok, &format!("已抢中“{}”,同课程其余 {} 个教学班已自动停用(one-per-no)。", no, n_dis));
                                }
                            }
                        }
                    }
                    Out::Full => { full_since.insert(t.id, cycle_no); }
                    Out::StopNoRetry | Out::Conflict => {
                        // 这类是确定性的失败,自动停用该目标避免无限空转
                        let mut g = data.lock().unwrap();
                        if let Some(x) = g.targets.iter_mut().find(|x| x.id == t.id) { x.enabled = false; }
                    }
                    Out::Expired => { break; } // 已记录日志,跳出主循环
                    _ => {}
                }
                ctx.request_repaint();
            }
        }

        // ---- 3) 全中自动停止 ----
        if data.lock().unwrap().auto_stop_done {
            let (enabled, won) = {
                let g = data.lock().unwrap();
                (
                    g.targets.iter().filter(|t| t.enabled).count(),
                    g.targets.iter().filter(|t| t.enabled && t.ok).count(),
                )
            };
            if enabled > 0 && won == enabled {
                log_msg(data, Lvl::Succ, format!("🎉 全部 {} 个目标已抢中,自动停止。", enabled));
                break;
            }
        }

        // ---- 4) 节奏 ----
        let used = cyc.elapsed().as_millis() as u64;
        let refresh_ms = data.lock().unwrap().refresh_ms.max(200);
        if used < refresh_ms { wait_ms(run, refresh_ms - used); }
    }
}

/// 同课程编号选一即停:把与 id 同 no 的其他已启用目标停用,返回停用数量。
fn disable_siblings(data: &Arc<Mutex<AppData>>, no: &str, id: u64) -> usize {
    let mut n = 0usize;
    let mut g = data.lock().unwrap();
    for t in g.targets.iter_mut() {
        if t.enabled && !t.ok && t.no == no && t.id != id {
            t.enabled = false;
            t.last = "同课程已抢到一门,自动停用".into();
            n += 1;
        }
    }
    n
}

fn target_name(data: &Arc<Mutex<AppData>>, id: &str) -> Option<String> {
    let g = data.lock().unwrap();
    g.targets.iter().find(|t| t.id.to_string() == id)
        .map(|t| if t.name.is_empty() { t.no.clone() } else { t.name.clone() })
}

fn wait_ms(run: &AtomicBool, ms: u64) {
    let step = 60u64;
    let mut left = ms;
    while left > 0 && run.load(Ordering::SeqCst) {
        let s = step.min(left);
        std::thread::sleep(Duration::from_millis(s));
        left -= s;
    }
}

enum Out {
    Ok,
    Full,
    Conflict,
    TooFast,
    Expired,
    StopNoRetry,
    NetErr,
    Unknown,
}

fn do_submit(api: &mut Api, base: &str, pid: &str, t: &Target, data: &Arc<Mutex<AppData>>, last_submit: &mut Instant) -> Out {
    *last_submit = Instant::now();
    {
        let mut g = data.lock().unwrap();
        if let Some(x) = g.targets.iter_mut().find(|x| x.id == t.id) {
            x.state = TState::Grabbing;
            x.tries += 1;
        }
    }
    let (spath, squery, sbody) = {
        let g = data.lock().unwrap();
        (g.submit_path.clone(), g.submit_query.clone(), g.submit_body.clone())
    };
    let url = format!("{}/eams/{}?{}",
        base.trim_end_matches('/'), spath.trim_start_matches('/'),
        squery.replace("{pid}", pid));
    let body = sbody.replace("{id}", &t.id.to_string()).replace("{pid}", pid);

    let resp = match api.post_form(&url, &body, true) {
        Ok(r) => r,
        Err(e) => {
            patch_target(data, t.id, |x| { x.state = TState::Idle; x.last = format!("网络错误:{}", e); });
            if t.tries % 8 == 1 { log_msg(data, Lvl::Warn, &format!("[{}] 提交网络错误:{}", t.id, e)); }
            return Out::NetErr;
        }
    };
    let kind = net::classify_submit(&resp.text, &resp.final_url);
    let tries = target_tries(data, t.id);
    match kind {
        net::SubKind::Ok => {
            patch_target(data, t.id, |x| { x.state = TState::Won; x.ok = true; x.last = "抢中 ✓".into(); });
            log_msg(data, Lvl::Succ, &format!("🎉 [{}] “{}” 抢课成功!", t.id, short_name(t)));
            Out::Ok
        }
        net::SubKind::Full => {
            patch_target(data, t.id, |x| { x.state = TState::Full; x.last = "容量已满(自动冷却后继续)".into(); });
            if tries % 8 == 1 { log_msg(data, Lvl::Warn, &format!("[{}] “{}” 暂时满员,继续蹲守。", t.id, short_name(t))); }
            Out::Full
        }
        net::SubKind::Conflict => {
            patch_target(data, t.id, |x| { x.state = TState::Conflict; x.last = "课程/时间冲突".into(); });
            log_msg(data, Lvl::Err, &format!("[{}] “{}” 提示冲突,已自动停用该目标,请人工核对。", t.id, short_name(t)));
            Out::Conflict
        }
        net::SubKind::TooFast => {
            patch_target(data, t.id, |x| { x.state = TState::Idle; x.last = "请求过快,已放缓节奏".into(); });
            Out::TooFast
        }
        net::SubKind::Expired => {
            patch_target(data, t.id, |x| { x.state = TState::SessionLost; x.last = "会话过期".into(); });
            log_msg(data, Lvl::Err, "会话已过期/被踢下线,引擎停止。请到浏览器重新登录后更新 Cookie。");
            Out::Expired
        }
        net::SubKind::Stop => {
            patch_target(data, t.id, |x| { x.state = TState::Blocked; x.last = "系统拒绝(未开放/无权限/不在选课时间)".into(); });
            log_msg(data, Lvl::Err, &format!("[{}] “{}” 被系统拒绝(可能未到选课时间或无权限),已自动停用。", t.id, short_name(t)));
            Out::StopNoRetry
        }
        net::SubKind::Unknown => {
            let snippet = net::strip_html(&resp.text);
            let msg = if snippet.is_empty() { format!("HTTP {}", resp.status) } else { snippet };
            patch_target(data, t.id, |x| { x.state = TState::Idle; x.last = msg.clone(); });
            if tries < 4 || tries % 10 == 1 {
                log_msg(data, Lvl::Warn, &format!("[{}] “{}” 返回未知,继续尝试:{}", t.id, short_name(t), msg));
            }
            {
                let mut g = data.lock().unwrap();
                g.last_raw = trunc(&resp.text, 500);
            }
            Out::Unknown
        }
    }
}

fn short_name(t: &Target) -> String {
    let mut s = String::new();
    if !t.no.is_empty() { s.push_str(&t.no); s.push(' '); }
    if t.name.is_empty() {
        s.push_str(&t.id.to_string());
    } else {
        s.push_str(&t.name);
    }
    s
}

fn patch_target(data: &Arc<Mutex<AppData>>, id: u64, f: impl FnOnce(&mut Target)) {
    let mut g = data.lock().unwrap();
    if let Some(t) = g.targets.iter_mut().find(|t| t.id == id) { f(t); }
}

fn target_tries(data: &Arc<Mutex<AppData>>, id: u64) -> u32 {
    let g = data.lock().unwrap();
    g.targets.iter().find(|t| t.id == id).map(|t| t.tries).unwrap_or(0)
}
