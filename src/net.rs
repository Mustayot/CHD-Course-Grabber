//! 网络层：手工 Cookie 会话 + URP/eams 接口 + 容错 JS 解析 + CAS 免密登录(best-effort)。
use crate::model;
use std::collections::BTreeMap;
use std::time::Duration;

pub struct Api {
    pub cli: reqwest::blocking::Client,
    pub jar: BTreeMap<String, String>, // cookie 名 -> 值(手工维护)
}

pub struct Resp {
    pub status: u16,
    pub text: String,
    pub final_url: String,
}

impl Api {
    pub fn new() -> Api {
        let cli = reqwest::blocking::Client::builder()
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
            .timeout(Duration::from_secs(15))
            .connect_timeout(Duration::from_secs(8))
            .gzip(true)
            .build()
            .expect("build http client");
        Api { cli, jar: BTreeMap::new() }
    }

    // ---------------- cookie ----------------
    pub fn set_cookie_str(&mut self, raw: &str) {
        self.jar.clear();
        for piece in raw.split(';') {
            let piece = piece.trim();
            if piece.is_empty() { continue; }
            if let Some((k, v)) = piece.split_once('=') {
                let k = k.trim().trim_start_matches("Cookie:").trim();
                if !k.is_empty() {
                    self.jar.insert(k.to_string(), v.trim().to_string());
                }
            }
        }
    }

    fn header(&self) -> String {
        self.jar.iter().map(|(k, v)| format!("{}={}", k, v)).collect::<Vec<_>>().join("; ")
    }

    fn absorb(&mut self, resp: &reqwest::blocking::Response) {
        let sets: Vec<&str> = resp.headers().get_all("set-cookie").iter()
            .filter_map(|v| v.to_str().ok()).collect();
        for s in sets {
            if let Some(eq) = s.find('=') {
                let name = s[..eq].trim().to_string();
                let end = s[eq + 1..].find(';').map(|i| eq + 1 + i).unwrap_or(s.len());
                let val = s[eq + 1..end].trim().to_string();
                if !name.is_empty() {
                    self.jar.insert(name, val);
                }
            }
        }
    }

    pub fn cookie_header(&self) -> String { self.header() }

    fn wrap_err(e: reqwest::Error) -> String {
        if e.is_timeout() { "请求超时(网络慢或教务系统繁忙)".into() }
        else if e.is_connect() { format!("无法连接服务器: {}", e) }
        else { format!("网络错误: {}", e) }
    }

    pub fn get(&mut self, url: &str) -> Result<Resp, String> {
        let r = self.cli.get(url).header("cookie", self.header())
            .send().map_err(|e| Self::wrap_err(e))?;
        let status = r.status().as_u16();
        let final_url = r.url().to_string();
        let text = r.text().unwrap_or_default();
        Ok(Resp { status, text, final_url })
    }

    pub fn post_form(&mut self, url: &str, body: &str, xhr: bool) -> Result<Resp, String> {
        let mut req = self.cli.post(url).header("cookie", self.header())
            .header("content-type", "application/x-www-form-urlencoded");
        if xhr { req = req.header("x-requested-with", "XMLHttpRequest"); }
        let r = req.body(body.to_string()).send().map_err(|e| Self::wrap_err(e))?;
        let status = r.status().as_u16();
        let final_url = r.url().to_string();
        let text = r.text().unwrap_or_default();
        Ok(Resp { status, text, final_url })
    }

    pub fn is_login_page(url: &str, body: &str) -> bool {
        let u = url.to_lowercase();
        (u.contains("login") || u.contains("authserver") || u.contains("/cas") || u.contains("ids."))
            && (body.contains("username") || body.contains("密码") || body.contains("password"))
    }
}

// ---------------------------------------------------------------------------
// 端点拼装
// ---------------------------------------------------------------------------
pub fn url_home(base: &str) -> String { format!("{}/eams/home.action", base) }
pub fn url_inner(base: &str) -> String { format!("{}/eams/stdElectCourse!innerIndex.action?projectId=1", base) }
/// 选课首页:列出当前学生所有可进入的选课轮次(每个轮次一个 defaultPage 链接)。
pub fn url_elect_home(base: &str) -> String { format!("{}/eams/stdElectCourse.action", base) }
pub fn url_default(base: &str, pid: &str) -> String {
    format!("{}/eams/stdElectCourse!defaultPage.action?electionProfile.id={}", base, pid)
}
pub fn url_data(base: &str, pid: &str) -> String {
    format!("{}/eams/stdElectCourse!data.action?profileId={}", base, pid)
}
pub fn url_data_for_lesson(base: &str, pid: &str, lesson_id: &str) -> String {
    format!("{}/eams/stdElectCourse!data.action?profileId={}&lesson.id={}", base, pid, lesson_id)
}
pub fn url_counts(base: &str, pid: &str, extra: &str) -> String {
    format!("{}/eams/stdElectCourse!queryStdCount.action?profileId={}{}", base, pid, extra)
}
pub fn url_counts_v2(base: &str, semester_id: &str, extra: &str) -> String {
    // 实测(eams):queryStdCount.action 需要 projectId + semesterId,而不是 profileId。
    // projectId 固定为 1;semesterId 从选课页 HTML 提取(默认兜底 262)。
    format!("{}/eams/stdElectCourse!queryStdCount.action?projectId=1&semesterId={}{}", base, semester_id, extra)
}
pub fn url_counts_for_lesson(base: &str, pid: &str, lesson_id: &str, extra: &str) -> String {
    format!("{}/eams/stdElectCourse!queryStdCount.action?profileId={}&lesson.id={}{}", base, pid, lesson_id, extra)
}

/// 从 HTML 提取 semesterId(eams 选课页内嵌的全局配置),供 queryStdCount 使用。
/// 兼容 "semesterId: 262" / "semesterId=262" / "semesterId:'262'" 等写法。
pub fn extract_semester_id(html: &str) -> Option<String> {
    let lower = html.to_lowercase();
    let mut idx = 0usize;
    while let Some(pos) = lower[idx..].find("semesterid") {
        let abs = idx + pos + "semesterid".len();
        let after = &html[abs..];
        // 允许紧跟 [ '" ]* 空白 : = 空白 再数字
        let mut chars = after.chars().peekable();
        let mut prefix = String::new();
        while let Some(&c) = chars.peek() {
            if c.is_whitespace() || c == ':' || c == '=' || c == '"' || c == '\'' || c == '\u{feff}' {
                prefix.push(c);
                chars.next();
            } else { break; }
        }
        let digits: String = chars.take_while(|c| c.is_ascii_digit()).collect();
        if !digits.is_empty() && prefix.chars().any(|c| c == ':' || c == '=') {
            return Some(digits);
        }
        idx = abs + 1;
    }
    None
}

// ---------------------------------------------------------------------------
// 从 defaultPage HTML 解析 lesson 列表(id + 课程名)。
// 兼容多种 eams 模板形式:
//   1) <option value="数字">课程名</option>
//   2) <a href="?lesson.id=数字">课程名</a>
//   3) <a onclick="...lesson.id=数字">课程名</a>
//   4) <li data-value="数字">课程名</li>
//   5) <input type="radio" name="lesson" value="数字"/> 课程名
// ---------------------------------------------------------------------------
pub fn parse_lesson_list(html: &str) -> Vec<(String, String)> {
    use std::collections::BTreeMap;
    let mut by_id: BTreeMap<String, String> = BTreeMap::new();
    let lower = html.to_lowercase();

    // ---- 1) <option value="数字">name</option> ----
    let mut i = 0;
    let bytes = html.as_bytes();
    while i < bytes.len() {
        if bytes[i..].starts_with(b"<option") {
            // 找到 '>' 结束标签头
            if let Some(et) = bytes[i..].iter().position(|&c| c == b'>') {
                let head_end = i + et + 1;
                let tag = &html[i..head_end];
                // 找 </option> (大小写不敏感)
                let tail = &lower[head_end..];
                if let Some(close) = tail.find("</option>") {
                    let name = html[head_end..head_end + close].trim().to_string();
                    if let Some(v) = attr_of(tag, "value") {
                        let v = v.trim();
                        if !v.is_empty() && v.chars().all(|c| c.is_ascii_digit()) {
                            by_id.entry(v.to_string()).or_insert(name);
                        }
                    }
                    i = head_end + close + "</option>".len();
                    continue;
                }
            }
        }
        i += 1;
    }

    // ---- 2) lesson.id=N 任意位置(链接/onclick/隐藏字段),取最近 <a> 之后的文字 ----
    let mut idx = 0usize;
    while let Some(pos) = lower[idx..].find("lesson.id=") {
        let abs = idx + pos + "lesson.id=".len();
        let id: String = html[abs..].chars().take_while(|c| c.is_ascii_digit()).collect();
        if !id.is_empty() {
            // 找这个 lesson.id 后面最近的 </a> 结束符
            if let Some(close) = lower[abs..].find("</a>") {
                let label_end = abs + close;
                // 找 <a 起点: 在 abs 之前最近的 <a 标签
                let before = &html[..abs];
                if let Some(a_rel) = before.to_lowercase().rfind("<a ") {
                    let a_start = a_rel;
                    // <a ...> 后的内容作为 label
                    if let Some(gt) = html[a_start..abs].find('>') {
                        let label = html[a_start + gt + 1..label_end].trim().to_string();
                        if !label.is_empty() && label.len() < 200 {
                            by_id.entry(id.clone()).or_insert(label);
                        }
                    }
                }
            } else {
                // 不是链接,可能是 onClick / JS 调用
                let before = &html[..abs];
                if let Some(q) = before.rfind('>') {
                    let after = &html[q + 1..abs];
                    if !after.is_empty() && after.len() < 200 && !after.contains('<') {
                        by_id.entry(id.clone()).or_insert(after.trim().to_string());
                    }
                }
            }
        }
        idx = abs + id.len().max(1);
    }

    // ---- 3) <li data-value="数字">课程名</li> (新模板) ----
    i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"<li") {
            if let Some(et) = bytes[i..].iter().position(|&c| c == b'>') {
                let head_end = i + et + 1;
                let tag = &html[i..head_end];
                if let Some(v) = attr_of(tag, "data-value") {
                    let v = v.trim();
                    if !v.is_empty() && v.chars().all(|c| c.is_ascii_digit()) {
                        if let Some(close) = lower[head_end..].find("</li>") {
                            let name = html[head_end..head_end + close].trim().to_string();
                            if !name.is_empty() {
                                by_id.entry(v.to_string()).or_insert(name);
                            }
                        }
                    }
                }
            }
        }
        i += 1;
    }

    by_id.into_iter().collect()
}

// ---------------------------------------------------------------------------
// 容错 JS 对象解析(兼容 JSON 与无引号键的 JS 字面量)
// ---------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub enum Val {
    Nul,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Val>),
    Obj(Vec<(String, Val)>),
}

struct P<'a> { b: &'a [u8], i: usize }

impl<'a> P<'a> {
    fn ws(&mut self) {
        while self.i < self.b.len() && self.b[self.i].is_ascii_whitespace() { self.i += 1; }
    }
    fn peek(&mut self) -> Option<u8> { self.ws(); self.b.get(self.i).copied() }
    fn val(&mut self) -> Option<Val> {
        self.ws();
        if self.i >= self.b.len() { return None; }
        let c = self.b[self.i];
        match c {
            b'{' => self.obj(),
            b'[' => self.arr(),
            b'"' | b'\'' => Some(Val::Str(self.str_tok(c).unwrap_or_default())),
            b't' => { self.word("true"); Some(Val::Bool(true)) }
            b'f' => { self.word("false"); Some(Val::Bool(false)) }
            b'n' => { self.word("null"); Some(Val::Nul) }
            _ => {
                let start = self.i;
                while self.i < self.b.len()
                    && !matches!(self.b[self.i], b',' | b'}' | b']' | b'{' | b'[' | b':' | b' ' | b'\t' | b'\n' | b'\r') {
                    self.i += 1;
                }
                let s = String::from_utf8_lossy(&self.b[start..self.i]).to_string();
                if s.is_empty() { None }
                else if let Ok(n) = s.parse::<f64>() { Some(Val::Num(n)) }
                else { Some(Val::Str(s)) } // 未加引号的裸串,尽力而为
            }
        }
    }
    fn word(&mut self, w: &str) {
        let n = w.len();
        self.i = (self.i + n).min(self.b.len());
    }
    fn obj(&mut self) -> Option<Val> {
        self.i += 1; // {
        let mut out = Vec::new();
        loop {
            self.ws();
            if self.i >= self.b.len() { break; }
            if self.b[self.i] == b'}' { self.i += 1; break; }
            // key
            let key = if self.b[self.i] == b'"' || self.b[self.i] == b'\'' {
                let q = self.b[self.i];
                self.str_tok(q).unwrap_or_default()
            } else {
                let s = self.i;
                while self.i < self.b.len() && self.b[self.i] != b':' { self.i += 1; }
                String::from_utf8_lossy(&self.b[s..self.i]).trim().to_string()
            };
            self.ws();
            if self.i < self.b.len() && self.b[self.i] == b':' { self.i += 1; }
            let before = self.i;
            let v = self.val().unwrap_or(Val::Nul);
            if self.i == before { self.i = (self.i + 1).min(self.b.len()); } // 防呆
            out.push((key, v));
            self.ws();
            if self.i < self.b.len() && self.b[self.i] == b',' { self.i += 1; }
        }
        Some(Val::Obj(out))
    }
    fn arr(&mut self) -> Option<Val> {
        self.i += 1; // [
        let mut out = Vec::new();
        loop {
            self.ws();
            if self.i >= self.b.len() { break; }
            if self.b[self.i] == b']' { self.i += 1; break; }
            let before = self.i;
            let v = self.val().unwrap_or(Val::Nul);
            if self.i == before { self.i = (self.i + 1).min(self.b.len()); } // 防呆
            out.push(v);
            self.ws();
            if self.i < self.b.len() && self.b[self.i] == b',' { self.i += 1; }
        }
        Some(Val::Arr(out))
    }
    fn str_tok(&mut self, q: u8) -> Option<String> {
        self.i += 1; // 吃掉开引号
        let mut s = Vec::new();
        while self.i < self.b.len() {
            let c = self.b[self.i]; self.i += 1;
            if c == b'\\' && self.i < self.b.len() {
                let e = self.b[self.i]; self.i += 1;
                match e {
                    b'n' => s.push(b'\n'),
                    b't' => s.push(b'\t'),
                    b'r' => s.push(b'\r'),
                    b'u' => {
                        if self.i + 4 <= self.b.len() {
                            if let Ok(hex) = std::str::from_utf8(&self.b[self.i..self.i + 4]) {
                                if let Ok(cp) = u32::from_str_radix(hex, 16) {
                                    if let Some(ch) = char::from_u32(cp) {
                                        let mut buf = [0u8; 4];
                                        s.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                                    }
                                }
                            }
                            self.i += 4;
                        }
                    }
                    other => s.push(other),
                }
            } else if c == q { break; }
            else { s.push(c); }
        }
        Some(String::from_utf8_lossy(&s).to_string())
    }
}

pub fn parse_val_start(text: &str, start: usize) -> Option<Val> {
    let b = text.as_bytes();
    if start >= b.len() { return None; }
    P { b, i: start }.val()
}

pub fn val_str(v: &Val) -> String {
    match v {
        Val::Str(s) => s.clone(),
        Val::Num(n) => {
            if n.fract() == 0.0 { format!("{}", *n as i64) } else { format!("{}", n) }
        }
        Val::Bool(b) => b.to_string(),
        Val::Arr(a) => a.iter().map(val_str).collect::<Vec<_>>().join(","),
        Val::Obj(_) => String::new(),
        Val::Nul => String::new(),
    }
}

pub fn val_u64(v: &Val) -> Option<u64> {
    match v {
        Val::Num(n) if *n >= 0.0 => Some(*n as u64),
        Val::Str(s) => s.trim().parse::<u64>().ok(),
        _ => None,
    }
}

fn obj_get<'x>(v: &'x Val, key: &str) -> Option<&'x Val> {
    if let Val::Obj(items) = v {
        items.iter().find(|(k, _)| k == key).map(|(_, val)| val)
    } else { None }
}

// ---------------------------------------------------------------------------
// 课程列表 data.action 解析
// ---------------------------------------------------------------------------
pub fn parse_course_list(text: &str) -> Vec<model::CourseRow> {
    use crate::model::CourseRow;
    let mut out = Vec::new();
    if let Some(vi) = text.find('[') {
        if let Some(v) = parse_val_start(text, vi) {
            if let Val::Arr(rows) = v {
                for r in rows {
                    let id = obj_get(&r, "id").and_then(val_u64).unwrap_or(0);
                    if id == 0 { continue; }
                    let no = obj_get(&r, "no").map(val_str).unwrap_or_default();
                    let name = obj_get(&r, "name").map(val_str).unwrap_or_default();
                    let teachers = obj_get(&r, "teachers").map(val_str).unwrap_or_default();
                    out.push(CourseRow { id, no, name, teachers, sc: None, lc: None });
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// 余量 window.lessonId2Counts 解析 -> id -> (sc,lc)
// ---------------------------------------------------------------------------
pub fn parse_counts(text: &str) -> BTreeMap<String, (u64, u64)> {
    let mut map = BTreeMap::new();
    let pos = text.find("lessonId2Counts")
        .or_else(|| text.find("lessonId2Count"))
        .or_else(|| text.find('{'));
    let Some(start) = pos else { return map };
    let seg = &text[start..];
    let brace = if seg.starts_with("lessonId2Counts") || seg.starts_with("lessonId2Count") {
        seg.find('{').map(|x| start + x)
    } else {
        None
    };
    let at = brace.unwrap_or(start);
    if let Some(v) = parse_val_start(text, at) {
        if let Val::Obj(items) = v {
            for (k, cv) in items {
                let sc = obj_get(&cv, "sc").and_then(val_u64).unwrap_or(0);
                let lc = obj_get(&cv, "lc").and_then(val_u64).unwrap_or(0);
                map.insert(k, (sc, lc));
            }
        }
    }
    map
}

// ---------------------------------------------------------------------------
// 提交结果分类
// ---------------------------------------------------------------------------
#[derive(PartialEq, Clone, Copy, Debug)]
pub enum SubKind { Ok, Full, Conflict, TooFast, Expired, Stop, Unknown }

pub fn classify_submit(text: &str, url: &str) -> SubKind {
    let t = text.to_lowercase();
    let u = url.to_lowercase();
    if u.contains("login") || t.contains("会话已经被过期") || t.contains("会话过期")
        || (t.contains("会话") && t.contains("失效")) || (t.contains("请") && t.contains("重新登录"))
        || t.contains("loginForm") {
        return SubKind::Expired;
    }
    if t.contains("已经选过") || t.contains("已选") && t.contains("成功")
        || t.contains("选课成功") || t.contains("成功选课")
        || t.contains("成功") && (t.contains("选课") || t.contains("加入")) {
        return SubKind::Ok;
    }
    if t.contains("上限") || t.contains("已满") || t.contains("满员")
        || t.contains("已达") || t.contains("容量") || t.contains("人数已满")
        || t.contains("爆满") { return SubKind::Full; }
    if t.contains("冲突") || t.contains("时间冲突") || t.contains("课程冲突") { return SubKind::Conflict; }
    if t.contains("过快") || t.contains("频率") || t.contains("频繁")
        || t.contains("稍后") || t.contains("请勿") { return SubKind::TooFast; }
    if t.contains("未开放") || t.contains("未开始") || t.contains("不允许")
        || t.contains("停止选课") || t.contains("不在选课") || t.contains("没有权限")
        || t.contains("不参与") || t.contains("不可选") || t.contains("停选") { return SubKind::Stop; }
    SubKind::Unknown
}

pub fn strip_html(t: &str) -> String {
    // 提取最像提示语的可见文本
    let mut s = t.to_string();
    let mut plain = String::new();
    let mut in_tag = false;
    for c in s.drain(..) {
        if c == '<' { in_tag = true; continue; }
        if c == '>' { in_tag = false; continue; }
        if !in_tag { plain.push(c); }
    }
    plain.split_whitespace().collect::<Vec<_>>().join(" ").trim().chars().take(200).collect()
}

// ---------------------------------------------------------------------------
// CAS/统一身份认证 免密尝试(尽力而为;带验证码/动态码的学校请走浏览器+复制Cookie)
// ---------------------------------------------------------------------------
fn attr_of(tag: &str, name: &str) -> Option<String> {
    // 在单个 <input ...> 中找 name="xxx" / name='xxx'
    let n = format!("{}=", name);
    let lower = tag.to_lowercase();
    let idx = lower.find(&n.to_lowercase())?;
    let rest = &tag[idx + n.len()..];
    let q = rest.chars().next()?;
    if q == '"' || q == '\'' {
        let end = rest[1..].find(q)?;
        Some(rest[1..1 + end].to_string())
    } else {
        let end = rest.find([' ', '>']).unwrap_or(rest.len());
        Some(rest[..end].to_string())
    }
}

/// 尝试自动登录。成功返回 "cookie1=v1; cookie2=v2"。失败返回 Err(人类可读原因)。
pub fn try_login(api: &mut Api, base: &str, user: &str, pass: &str) -> Result<String, String> {
    if user.trim().is_empty() || pass.is_empty() {
        return Err("未填写学号或密码".into());
    }
    let service = format!("{}/eams/login.action", base);

    // 1) 看 eams 是否把未登录请求踢到 CAS
    let probe = api.get(&service).map_err(|e| e)?;
    let mut cas_login_url: Option<String> = None;

    if probe.status == 302 || probe.status == 301 || probe.status == 303 {
        // 需要手动跟随才能拿到 Location —— 重新用 no-redirect 模式探测
        return Err(format!("该教务站点登录涉及多级跳转({}),请改用“浏览器登录后复制 Cookie”方式。", probe.status));
    }
    if Api::is_login_page(&probe.final_url, &probe.text) {
        // 已经在登录页(可能是 eams 自己的,也可能是 CAS 落地页)
        if probe.final_url.contains("authserver") || probe.final_url.contains("login") {
            cas_login_url = Some(probe.final_url.clone());
        } else {
            cas_login_url = Some(probe.final_url.clone());
        }
    } else {
        // 直接就是登录页/或已登录
        if probe.status == 200 && !probe.text.contains("username") && !probe.text.contains("password") {
            return Ok(api.cookie_header());
        }
        cas_login_url = Some(probe.final_url.clone());
    }

    let login_url = cas_login_url.ok_or("无法定位统一身份认证登录地址")?;

    // 2) 拉取登录表单,收集隐藏字段
    let form_resp = api.get(&login_url).map_err(|e| e)?;
    let body = &form_resp.text;
    if body.contains("captcha") || body.contains("vcode") || body.contains("验证码")
        || body.contains("动态码") || body.contains("扫码") || body.contains("qrCode") {
        return Err("登录页需要验证码/动态码/扫码,自动登录不可用;请在浏览器完成登录后,把 Cookie 粘贴到本程序。".into());
    }

    // 收集 <input name= value=>
    let mut fields: Vec<(String, String)> = Vec::new();
    let mut user_key = "username".to_string();
    let mut pass_key = "password".to_string();
    let bytes = body.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"<input") {
            let end = bytes[i..].iter().position(|&c| c == b'>').map(|x| i + x).unwrap_or(bytes.len());
            let tag = String::from_utf8_lossy(&bytes[i..end]).to_string();
            let lower = tag.to_lowercase();
            if let Some(n) = attr_of(&tag, "name") {
                let lname = n.to_lowercase();
                if lname.contains("user") || lname.contains("account") || lname.contains("j_username") {
                    user_key = n; continue;
                }
                if lname.contains("pass") || lname.contains("pwd") || lname.contains("mm") {
                    pass_key = n; continue;
                }
                // 隐藏字段原样带回
                if lower.contains("type=\"hidden\"") || lower.contains("type='hidden'") {
                    if let Some(v) = attr_of(&tag, "value") {
                        fields.push((n, v));
                    }
                }
            }
            i = end;
        }
        i += 1;
    }
    // 防止把密码又当隐藏值塞进去
    fields.retain(|(k, _)| !k.eq_ignore_ascii_case(&user_key) && !k.eq_ignore_ascii_case(&pass_key));
    fields.push((user_key.to_string(), user.to_string()));
    fields.push((pass_key.to_string(), pass.to_string()));

    // 3) 提交表单(带 service 参数,跟随重定向直至回到教务)
    let encoded: Vec<String> = fields.iter()
        .map(|(k, v)| format!("{}={}", urlenc(k), urlenc(v)))
        .collect();
    let body_s = encoded.join("&");
    let post_url = if login_url.contains('?') {
        format!("{}&service={}", login_url, urlenc(&service))
    } else {
        format!("{}?service={}", login_url, urlenc(&service))
    };
    let resp = api.post_form(&post_url, &body_s, false)?;

    if Api::is_login_page(&resp.final_url, &resp.text) {
        let hint = strip_html(&resp.text);
        let hint = if hint.len() > 120 { hint[..120].to_string() } else { hint };
        return Err(format!("登录未成功(可能账号/密码错误或需要验证码)。页面提示:{}", hint));
    }
    // 验证会话
    let home = api.get(&url_home(base)).map_err(|e| e)?;
    if home.status == 200 && !Api::is_login_page(&home.final_url, &home.text) {
        Ok(api.cookie_header())
    } else {
        Err("登录链已跳转但会话未生效,建议改用浏览器登录后复制 Cookie。".into())
    }
}

pub fn urlenc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}
