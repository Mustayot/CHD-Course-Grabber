//! 体育课(stuh5 / 智慧体育 SaaS)专用网络 + 抢课引擎。
//! 与 eams 的 Api(手工 Cookie)不同,这里每个账号用独立 Bearer token + tenant-id。
use crate::model::{AppData, Lvl, SportClass};
use chrono::Timelike;
use eframe::egui;
use reqwest::blocking::Client;
use reqwest::header::HeaderMap;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const SPORT_BASE: &str = "https://stuh5.chd.edu.cn/admin-api";
pub const SPORT_SVC: &str = "anonymous/h5/teaching/teaching-course-arrange";
const TENANT: &str = "544";

fn client(token: &str) -> Client {
    let mut hm = HeaderMap::new();
    hm.insert("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64)".parse().unwrap());
    hm.insert("Authorization", format!("Bearer {}", token).parse().unwrap());
    hm.insert("tenant-id", TENANT.parse().unwrap());
    Client::builder()
        .timeout(Duration::from_secs(10))
        .no_proxy()
        .default_headers(hm)
        .build()
        .expect("sport client")
}

fn json_get(token: &str, path: &str) -> Result<serde_json::Value, String> {
    let url = format!("{}/{}", SPORT_BASE, path);
    let r = client(token).get(&url).send().map_err(|e| e.to_string())?;
    let txt = r.text().unwrap_or_default();
    serde_json::from_str::<serde_json::Value>(&txt).map_err(|e| format!("json:{}", e))
}

fn code_of(v: &serde_json::Value) -> i64 {
    v.get("code").and_then(|x| x.as_i64()).unwrap_or(-1)
}
fn msg_of(v: &serde_json::Value) -> String {
    v.get("msg").and_then(|x| x.as_str()).map(|s| s.to_string()).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// 课程库
// ---------------------------------------------------------------------------
pub fn load_library(token: &str) -> Result<Vec<SportClass>, String> {
    let v = json_get(token, &format!("{}/student-course-arrange-page?pageNo=1&pageSize=500", SPORT_SVC))?;
    if code_of(&v) != 0 {
        return Err(format!("code={} {}", code_of(&v), msg_of(&v)));
    }
    let mut out = Vec::new();
    if let Some(list) = v.pointer("/data/list").and_then(|x| x.as_array()) {
        for c in list {
            let id = c.get("id").and_then(|x| x.as_i64()).unwrap_or(0);
            if id == 0 { continue; }
            let g = |k: &str| c.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
            let n = g("courseName");
            let cn = g("courseInfoName");
            let name = if cn.is_empty() || cn == n { n } else { format!("{}{}", n, cn) };
            let teachers = c.get("teacherNames").and_then(|x| x.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(" "))
                .unwrap_or_default();
            let used = c.get("totalReservedNumber").and_then(|x| x.as_u64()).unwrap_or(0);
            let cap = c.get("totalStudents").and_then(|x| x.as_u64()).unwrap_or(0);
            let sex = match c.get("classType").and_then(|x| x.as_str()).unwrap_or("") {
                "MALE_CLASS" => "男",
                "FEMALE_CLASS" => "女",
                "PROPORTION_CLASS" => "比",
                _ => "混",
            };
            out.push(SportClass {
                id: id as u64,
                name,
                time: g("classTime"),
                teacher: teachers,
                loc: g("location"),
                used,
                cap,
                sex: sex.into(),
            });
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// 查询我的选课记录(入册铁证)。token 失效时 Err 以 "401" 开头。
// ---------------------------------------------------------------------------
pub fn check_scores(token: &str) -> Result<Vec<String>, String> {
    let v = json_get(token, &format!("{}/student-course-scores", SPORT_SVC))?;
    if code_of(&v) != 0 {
        if code_of(&v) == 401 { return Err("401 token 失效".into()); }
        return Err(format!("code={} {}", code_of(&v), msg_of(&v)));
    }
    let mut notes = Vec::new();
    if let Some(scores) = v.pointer("/data/scores").and_then(|x| x.as_array()) {
        for s in scores {
            let g = |k: &str| s.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
            let a = g("courseInfoName");
            let b = g("classTimeName");
            let t = g("teacherName");
            notes.push(if t.is_empty() { format!("{} {}", a, b) } else { format!("{} {}({})", a, b, t) });
        }
    }
    Ok(notes)
}

// ---------------------------------------------------------------------------
// 提交预约
// ---------------------------------------------------------------------------
#[derive(Debug, Clone, PartialEq)]
pub enum TryOut {
    Ok,          // 提交成功 / 提示“已预约”(都视为已占位)
    Full,        // 已满 -> 顺延下一志愿
    Stop,        // 未开放/未到时间
    BadToken,    // 401
    NetErr(String),
    Unknown(String),
}

pub fn do_reserve(token: &str, id: u64) -> TryOut {
    let url = format!("{}/{}/reserve?arrangeId={}", SPORT_BASE, SPORT_SVC, id);
    let r = client(token).post(&url).send();
    let txt = match r {
        Ok(r) => r.text().unwrap_or_default(),
        Err(e) => return TryOut::NetErr(e.to_string()),
    };
    let v = serde_json::from_str::<serde_json::Value>(&txt).unwrap_or_default();
    let code = code_of(&v);
    let msg = msg_of(&v);
    if code == 0 { return TryOut::Ok; }
    if code == 401 { return TryOut::BadToken; }
    if ["满", "上限", "不可约", "已满", "人满"].iter().any(|k| msg.contains(k)) { return TryOut::Full; }
    if ["未到", "未开放", "不在", "时间未", "还没开始", "不允许", "超出"].iter().any(|k| msg.contains(k)) {
        return TryOut::Stop;
    }
    if ["已选", "已预约", "重复", "已选过", "已约"].iter().any(|k| msg.contains(k)) { return TryOut::Ok; }
    if msg.is_empty() { TryOut::Unknown(format!("HTTP/empty (code={})", code)) } else { TryOut::Unknown(msg) }
}

// ---------------------------------------------------------------------------
// 引擎主循环(在独立线程运行)
// ---------------------------------------------------------------------------
pub fn run_engine(ctx: egui::Context, data: Arc<Mutex<AppData>>) {
    std::thread::spawn(move || {
        let (fire_sec, gate, interval_ms) = {
            let g = data.lock().unwrap();
            (parse_fire(&g.sport_fire), g.sport_gate_fire, g.sport_interval_ms.max(150))
        };
        {
            let mut g = data.lock().unwrap();
            g.sport_active = true;
            g.sport_msg = "▶ 引擎运行中 · 未入册即抢 / 已入册盯防 / 到点确认".into();
            g.log(Lvl::Info, "体育引擎启动:未入册账号按志愿序连发;已入册账号盯防记录(一旦被清立即抢回);到开抢时刻会做一次通道确认。");
            for a in g.sport_accounts.iter_mut() {
                if a.st != "err" && a.st != "ok" { a.st = "armed".into(); }
            }
        }

        let mut last_check: HashMap<usize, Instant> = HashMap::new();
        let mut last_try: HashMap<usize, Instant> = HashMap::new();
        let mut enrolled: HashMap<usize, bool> = HashMap::new();
        let mut confirmed_fire: HashMap<usize, bool> = HashMap::new();
        let mut all_done_since: Option<Instant> = None;

        loop {
            if !data.lock().unwrap().sport_active { break; }
            hot_reload_cfg(&ctx, &data); // 外部改写 cfg.json 的 token → 每轮热切换
            let now = chrono::Local::now();
            let now_sec = now.num_seconds_from_midnight() as i64;
            let fire_reached = fire_sec.map(|f| now_sec >= f).unwrap_or(false);
            let fire_gate_ok = fire_reached || !gate;
            let t0 = Instant::now();

            let n_accs = { data.lock().unwrap().sport_accounts.len() };

            for ai in 0..n_accs {
                let snap = {
                    let g = data.lock().unwrap();
                    if ai >= g.sport_accounts.len() { continue; }
                    let a = &g.sport_accounts[ai];
                    (a.st.clone(), a.token.clone(), a.parsed_ids(), a.name.clone())
                };
                let (st0, token, ids, name) = snap;
                if st0 == "err" || st0 == "fail" { continue; }
                let mut is_ok = enrolled.get(&ai).copied().unwrap_or(false);

                // ---- 周期查入册(启动即查,之后每 8s) ----
                let need_check = last_check.get(&ai).map(|t| t.elapsed().as_secs() >= 8).unwrap_or(true);
                if need_check {
                    last_check.insert(ai, Instant::now());
                    match check_scores(&token) {
                        Ok(recs) => {
                            let now_enrolled = !recs.is_empty();
                            let prev = enrolled.insert(ai, now_enrolled).unwrap_or(false);
                            if now_enrolled {
                                let desc = recs[0].clone();
                                let mut g = data.lock().unwrap();
                                if ai < g.sport_accounts.len() {
                                    let a = &mut g.sport_accounts[ai];
                                    a.st = "ok".into();
                                    a.sub = format!("✅ 已入册:{}", desc);
                                    if !prev {
                                        g.log(Lvl::Succ, format!("🏸 {} 入册确认:{}", name, desc));
                                    }
                                }
                            } else if prev {
                                // 记录曾存在后消失 -> 立即抢回
                                let mut g = data.lock().unwrap();
                                if ai < g.sport_accounts.len() {
                                    g.sport_accounts[ai].st = "run".into();
                                    g.sport_accounts[ai].sub = "⚠️ 记录消失,立即抢回…".into();
                                    g.log(Lvl::Warn, format!("⚠️ {} 入册记录消失,立即抢回…", name));
                                }
                                last_try.insert(ai, Instant::now());
                            }
                        }
                        Err(e) if e.starts_with("401") => {
                            let mut g = data.lock().unwrap();
                            if ai < g.sport_accounts.len() {
                                g.sport_accounts[ai].st = "err".into();
                                g.sport_accounts[ai].sub = "token 失效,请换新".into();
                            }
                            g.log(Lvl::Err, format!("{}:token 失效(401),请重新取 token 粘贴后重启引擎。", name));
                            continue;
                        }
                        Err(_) => {}
                    }
                }

                is_ok = enrolled.get(&ai).copied().unwrap_or(false);

                // ---- 未入册 -> 尝试按志愿提交 ----
                if !is_ok && !ids.is_empty() && fire_gate_ok {
                    let gap = last_try.get(&ai).map(|t| t.elapsed().as_millis() as u64).unwrap_or(u64::MAX);
                    if gap >= interval_ms {
                        last_try.insert(ai, Instant::now());
                        let mut tried_id = 0u64;
                        let mut out = TryOut::Unknown("no-id".into());
                        for id in &ids {
                            tried_id = *id;
                            let r = do_reserve(&token, *id);
                            match r {
                                TryOut::Ok => { out = TryOut::Ok; break; }
                                TryOut::BadToken => { out = TryOut::BadToken; break; }
                                TryOut::Full => { out = TryOut::Full; continue; }
                                other => { out = other; break; }
                            }
                        }
                        // 提交成功 -> 立刻同步复核入册(入册可能滞后数秒)
                        if out == TryOut::Ok {
                            std::thread::sleep(Duration::from_millis(2200));
                            match check_scores(&token) {
                                Ok(recs) if !recs.is_empty() => {
                                    let desc = recs[0].clone();
                                    let mut g = data.lock().unwrap();
                                    if ai < g.sport_accounts.len() {
                                        g.sport_accounts[ai].st = "ok".into();
                                        g.sport_accounts[ai].sub = format!("✅ 已入册:{}", desc);
                                    }
                                    g.log(Lvl::Succ, format!("🏸 {} 预约成功并入册:{}", name, desc));
                                    enrolled.insert(ai, true);
                                    continue;
                                }
                                _ => {
                                    let mut g = data.lock().unwrap();
                                    if ai < g.sport_accounts.len() {
                                        g.sport_accounts[ai].st = "run".into();
                                        g.sport_accounts[ai].sub = "预约已提交(排队),等入册确认…".into();
                                    }
                                    // 入册未即时出现:冷却 12s 再复查,避免连发重复
                                    last_try.insert(ai, Instant::now() + Duration::from_secs(12));
                                }
                            }
                        } else {
                            let cls = lib_name(&data, tried_id);
                            let mut g = data.lock().unwrap();
                            if ai >= g.sport_accounts.len() { continue; }
                            match &out {
                                TryOut::BadToken => {
                                    g.sport_accounts[ai].st = "err".into();
                                    g.sport_accounts[ai].sub = "token 失效".into();
                                    g.log(Lvl::Err, format!("{}:token 失效(401),请换新 token。", name));
                                }
                                TryOut::Full => {
                                    g.sport_accounts[ai].sub = "志愿已满,顺延轮询(等退课)".into();
                                    g.sport_accounts[ai].st = "run".into();
                                }
                                TryOut::Stop => {
                                    g.sport_accounts[ai].sub = "未到开放时间,蹲守中".into();
                                    g.sport_accounts[ai].st = "run".into();
                                }
                                TryOut::NetErr(m) => {
                                    g.sport_accounts[ai].sub = format!("网络异常:{}", m);
                                    g.sport_accounts[ai].st = "run".into();
                                }
                                _ => {
                                    g.sport_accounts[ai].sub = format!("提交被拒:{} (#{})", out_str(&out), tried_id);
                                    g.sport_accounts[ai].st = "run".into();
                                }
                            }
                            let _ = cls;
                        }
                    }
                } else if is_ok {
                    // ---- 已入册 -> 到点做一次通道确认(无害,若被清则下轮复查发现并抢回) ----
                    if fire_reached && !confirmed_fire.get(&ai).copied().unwrap_or(false) {
                        confirmed_fire.insert(ai, true);
                        if let Some(first) = ids.first().copied() {
                            let r = do_reserve(&token, first);
                            let mut g = data.lock().unwrap();
                            if ai >= g.sport_accounts.len() { continue; }
                            let txt: String = match r {
                                TryOut::Ok => "✅ 到点通道确认通过(已在册)".into(),
                                TryOut::BadToken => { g.sport_accounts[ai].st = "err".into(); "token 失效".into() }
                                _ => "到点确认返回异常,引擎继续盯防".into(),
                            };
                            g.sport_accounts[ai].sub = txt.clone();
                            g.log(Lvl::Ok, format!("⏰ {}:{}", name, txt));
                        }
                    }
                }
            }

            // ---- 收工:到点后全员 ok/err 且确认完 -> 45s 后自动停 ----
            {
                let g = data.lock().unwrap();
                let has_open = g.sport_accounts.iter().any(|a| a.st == "run" || a.st == "armed");
                let all_done = !g.sport_accounts.is_empty()
                    && g.sport_accounts.iter().all(|a| a.st == "ok" || a.st == "err" || a.st == "fail");
                drop(g);
                if fire_reached && all_done && !has_open {
                    let s = all_done_since.get_or_insert(Instant::now());
                    if s.elapsed().as_secs() >= 45 {
                        let mut g = data.lock().unwrap();
                        g.sport_active = false;
                        g.sport_msg = "✅ 到点确认完成,引擎已收工".into();
                        g.log(Lvl::Ok, "体育引擎收工:全部账号已确认在册。");
                        drop(g);
                        ctx.request_repaint();
                        break;
                    }
                } else {
                    all_done_since = None;
                }
            }

            // ---- 节奏 ----
            let used = t0.elapsed().as_millis() as u64;
            let base_ms = interval_ms.min(1500).max(250);
            if used < base_ms { std::thread::sleep(Duration::from_millis(base_ms - used)); }
            ctx.request_repaint();
        }

        let mut g = data.lock().unwrap();
        g.sport_active = false;
        if !g.sport_msg.contains("收工") && !g.sport_msg.contains("空闲") {
            g.sport_msg = "已停止".into();
        }
    });
}

fn out_str(o: &TryOut) -> String {
    match o {
        TryOut::Ok => "成功".into(),
        TryOut::Full => "满员".into(),
        TryOut::Stop => "未开放".into(),
        TryOut::BadToken => "401".into(),
        TryOut::NetErr(m) => m.clone(),
        TryOut::Unknown(m) => m.clone(),
    }
}

fn lib_name(data: &Arc<Mutex<AppData>>, id: u64) -> String {
    let g = data.lock().unwrap();
    g.sport_lib.iter().find(|c| c.id == id)
        .map(|c| format!("{} {}", c.name, c.time))
        .unwrap_or_else(|| format!("#{}", id))
}

fn parse_fire(s: &str) -> Option<i64> {
    let (h, m) = s.trim().split_once(':')?;
    Some(h.trim().parse::<i64>().ok()? * 3600 + m.trim().parse::<i64>().ok()? * 60)
}

/// 热重载:外部把新 token 直接写进 cfg.json 的 sport_accounts,引擎每轮自动检测并换用(免重启)。
/// 换 token 后若该账号处于 err/fail,自动转 armed 重试。
fn hot_reload_cfg(ctx: &egui::Context, data: &Arc<Mutex<AppData>>) {
    let (path, cur) = {
        let g = data.lock().unwrap();
        if g.cfg_path.is_empty() { return; }
        let cur = g.sport_accounts.iter().map(|a| (a.name.clone(), a.token.clone())).collect::<Vec<_>>();
        (g.cfg_path.clone(), cur)
    };
    let Ok(s) = std::fs::read_to_string(&path) else { return };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) else { return };
    let Some(arr) = v.get("sport_accounts").and_then(|x| x.as_array()) else { return };
    let mut sw: Vec<String> = Vec::new();
    for e in arr {
        let (Some(name), Some(tok)) = (
            e.get("name").and_then(|x| x.as_str()),
            e.get("token").and_then(|x| x.as_str()),
        ) else { continue };
        let tok = tok.trim();
        if tok.is_empty() { continue; }
        if cur.iter().any(|(n, t)| n == name && t == tok) { continue; }
        let ids_str = e.get("ids_str").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let note = e.get("note").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let mut g = data.lock().unwrap();
        for a in g.sport_accounts.iter_mut() {
            if a.name == name && a.token != tok {
                a.token = tok.to_string();
                if !ids_str.is_empty() { a.ids_str = ids_str.clone(); }
                if !note.is_empty() { a.note = note.clone(); }
                if a.st == "err" || a.st == "fail" {
                    a.st = "armed".into();
                    a.sub = "token 已热更新,自动重试…".into();
                }
                sw.push(name.to_string());
            }
        }
    }
    if !sw.is_empty() {
        let mut g = data.lock().unwrap();
        g.log(Lvl::Ok, format!("♻️ token 热更新生效:{}", sw.join(" / ")));
        drop(g);
        ctx.request_repaint();
    }
}

/// 手动对单个账号执行一次“按志愿填到成功/满为止”,并回写状态。返回给人看的提示。
pub fn manual_fill(data: &Arc<Mutex<AppData>>, ai: usize) -> String {
    let (name, token, ids) = {
        let g = data.lock().unwrap();
        if ai >= g.sport_accounts.len() { return "无效账号".into(); }
        let a = &g.sport_accounts[ai];
        (a.name.clone(), a.token.clone(), a.parsed_ids())
    };
    if ids.is_empty() { return "志愿列表为空,请先填班级 id".into(); }
    let mut last_full = false;
    for id in &ids {
        match do_reserve(&token, *id) {
            TryOut::Ok => {
                std::thread::sleep(Duration::from_millis(2200));
                let recs = check_scores(&token).unwrap_or_default();
                let mut g = data.lock().unwrap();
                if ai < g.sport_accounts.len() {
                    if let Some(first) = recs.first() {
                        g.sport_accounts[ai].st = "ok".into();
                        g.sport_accounts[ai].sub = format!("✅ 已入册:{}", first);
                        g.log(Lvl::Succ, format!("🏸 {} 手动预约成功并入册:{}", name, first));
                    } else {
                        g.sport_accounts[ai].st = "run".into();
                        g.sport_accounts[ai].sub = "已提交(排队),等待入册确认…".into();
                    }
                }
                return format!("{} 预约提交成功。", name);
            }
            TryOut::BadToken => {
                let mut g = data.lock().unwrap();
                if ai < g.sport_accounts.len() {
                    g.sport_accounts[ai].st = "err".into();
                    g.sport_accounts[ai].sub = "token 失效".into();
                }
                return format!("{} token 失效(401)。", name);
            }
            TryOut::Full => { last_full = true; continue; }
            TryOut::Stop => {
                return format!("{} 该班未到开放时间,已蹲守。", name);
            }
            TryOut::NetErr(m) => return format!("{} 网络错误:{}", name, m),
            TryOut::Unknown(m) => return format!("{} 被拒:{}", name, m),
        }
    }
    if last_full {
        let mut g = data.lock().unwrap();
        if ai < g.sport_accounts.len() { g.sport_accounts[ai].sub = "志愿已满,等退课/顺延".into(); }
        return format!("{} 志愿当前全满。", name);
    }
    "未执行".into()
}
