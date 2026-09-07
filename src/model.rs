//! 数据模型：日志、课程行、抢课目标、策略与全局共享状态。
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

pub const APP_TITLE: &str = "URP/eams 选课助手 · 抢课 / 蹲守";
pub const DEFAULT_BASE: &str = "http://bkjw.chd.edu.cn";
pub const DEFAULT_PROFILE: &str = ""; // 留空:点「自动发现轮次」自动获取

// ---------------------------------------------------------------------------
// 日志
// ---------------------------------------------------------------------------
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Lvl {
    Info,
    Ok,
    Warn,
    Err,
    Succ,
}

impl Lvl {
    pub fn tag(&self) -> &'static str {
        match self {
            Lvl::Info => "信息",
            Lvl::Ok => "成功",
            Lvl::Warn => "警告",
            Lvl::Err => "错误",
            Lvl::Succ => "抢中",
        }
    }
}

#[derive(Clone)]
pub struct LogLine {
    pub t: String,   // HH:MM:SS
    pub lvl: Lvl,
    pub msg: String,
}

// ---------------------------------------------------------------------------
// 课程行（教学班）
// ---------------------------------------------------------------------------
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct CourseRow {
    pub id: u64,          // eams lessonId，抢课/余量都用它
    pub no: String,       // 课程编号
    pub name: String,     // 课程名
    pub teachers: String,
    pub sc: Option<u64>,  // 已选
    pub lc: Option<u64>,  // 容量
}

impl CourseRow {
    pub fn full_hit(&self) -> bool {
        match (self.sc, self.lc) {
            (Some(s), Some(l)) => s >= l,
            _ => false,
        }
    }
    pub fn display(&self) -> String {
        let mut s = String::new();
        if !self.no.is_empty() { s.push_str(&self.no); s.push_str(" · "); }
        s.push_str(&self.name);
        if !self.teachers.is_empty() { s.push_str(&format!("  [{}]", self.teachers)); }
        s
    }
}

// ---------------------------------------------------------------------------
// 抢课目标（一个目标 = 一个教学班）
// ---------------------------------------------------------------------------
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum TState {
    #[default]
    Idle,
    Grabbing,
    Won,          // 抢中/已选过
    Full,         // 已满(停止攻击该课,自动冷却)
    Conflict,     // 冲突等原因停手
    Blocked,      // 其他不可选
    SessionLost,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Target {
    pub id: u64,
    pub no: String,
    pub name: String,
    pub enabled: bool,
    pub sc: Option<u64>,
    pub lc: Option<u64>,
    #[serde(skip)]
    pub state: TState,
    #[serde(skip)]
    pub tries: u32,
    #[serde(skip)]
    pub last: String,
    #[serde(skip)]
    pub ok: bool,           // 最终成功
}

impl Target {
    pub fn new(id: u64, no: &str, name: &str) -> Self {
        Target { id, no: no.into(), name: name.into(), enabled: true,
                 sc: None, lc: None, state: TState::Idle, tries: 0,
                 last: String::new(), ok: false }
    }
    pub fn cap_hit(&self) -> bool {
        match (self.sc, self.lc) {
            (Some(s), Some(l)) => s >= l,
            _ => false,
        }
    }
    pub fn free(&self) -> bool {
        match (self.sc, self.lc) {
            (Some(s), Some(l)) => s < l,
            _ => false,
        }
    }
}

// ---------------------------------------------------------------------------
// 策略
// ---------------------------------------------------------------------------
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Strategy {
    Free,   // 蹲守: 有余量才提交
    Rush,   // 抢课: 无视余量直接循环提交
    Watch,  // 蹲守·仅监控: 只刷新余量,不提交
}

impl Strategy {
    pub fn label(&self) -> &'static str {
        match self {
            Strategy::Free => "蹲守抢漏 · 有余才抢",
            Strategy::Rush => "抢课模式 · 猛攻直抢",
            Strategy::Watch => "蹲守·仅监控余量",
        }
    }
    pub fn desc(&self) -> &'static str {
        match self {
            Strategy::Free => "低压力循环刷新余量,发现空位(有人退课)立刻提交,直到抢中。适合退补选/蹲守阶段。",
            Strategy::Rush => "选课窗口开启瞬间以最快节奏直接提交,不等余量变化。适合直选阶段开始时刻。",
            Strategy::Watch => "只查询已选/容量并打日志,当出现空位时只提醒不提交,手动确认后再换模式。",
        }
    }
}

// ---------------------------------------------------------------------------
// 全局共享状态(UI 与引擎线程共用)
// ---------------------------------------------------------------------------
#[derive(Clone)]
pub struct AppData {
    // 会话
    pub base: String,
    pub profile: String,
    pub cookie: String,
    pub cookie_visible: bool,
    pub account: String,
    pub password: String,
    pub session_msg: String,      // 会话测试结果
    pub elections: Vec<(String, String)>, // (name,id)

    // 检索
    pub keyword: String,
    pub quick_kw: String,     // 快捷自动抢关键词
    pub courses: Vec<CourseRow>,

    // 内嵌浏览器登录
    pub login_state: String,  // 空闲 / 登录窗口已打开 / 等待认证… / 已获取Cookie / 已取消 / 失败:…
    pub login_open: bool,     // 内置浏览器窗口是否打开
    pub auto_fetch_after_login: bool, // 登录成功自动拉取课程列表
    // UI 在渲染(持 data 锁)期间点击"打开内置浏览器"只能置此标志,
    // 真正打开动作由 update 下一帧在持锁前执行 —— open_login_window 内部会再次 lock,
    // 若在持锁期间直接调用会因同一线程重复加锁而永久死锁(界面"未响应")。
    pub pending_open_login: bool,

    // 目标
    pub targets: Vec<Target>,

    // 引擎
    pub strategy: Strategy,
    pub refresh_ms: u64,
    pub submit_ms: u64,
    pub auto_stop_done: bool,
    pub one_per_no: bool,     // 同课程编号抢到一门后自动停用其余教学班
    pub running: bool,
    pub engine_label: String,     // 引擎当前状态文字
    pub rounds: u64,
    pub started_at: Option<String>,

    // 高级
    pub submit_path: String,   // 相对 action,如 stdElectCourse!batchOperator.action
    pub submit_query: String,  // query 模板,{pid} 替换批次id
    pub submit_body: String,   // 表单体模板,{id} 替换教学班id,{pid} 替换批次id
    pub count_extra: String,   // queryStdCount 附加查询串(以 & 开头)
    pub semester_id: String,   // queryStdCount 用的 semesterId(从选课页自动提取,可在 UI 改)
    pub data_path: String,

    // 体育(Sport 页签)
    pub sport_lib: Vec<SportClass>,     // 课程库(id -> 班)
    pub sport_accounts: Vec<SportAcc>,  // 各账号配置(含 token/志愿)
    pub sport_fire: String,             // 开抢时刻 HH:MM
    pub sport_interval_ms: u64,         // 单号重试间隔
    pub sport_gate_fire: bool,          // true=到开抢时刻才允许提交; false=未入册立即尝试
    pub sport_auto_arm: bool,           // 程序启动时自动启动体育引擎(免手动点启动)
    pub sport_active: bool,             // 体育引擎是否运行
    pub sport_msg: String,              // 页面顶部状态
    pub sport_kw: String,               // 课程库过滤关键词

    // 杂项
    pub logs: VecDeque<LogLine>,
    pub last_raw: String,
    pub tab: Tab,
    pub curl_input: String,
    pub cfg_path: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Session,
    Search,
    Center,
    Log,
    Advanced,
    Sport,
}

// ---------------------------------------------------------------------------
// 体育课(智慧体育 SaaS / stuh5)专属数据
// ---------------------------------------------------------------------------

/// 体育教学班(课程库条目,来自 student-course-arrange-page)
#[derive(Clone)]
pub struct SportClass {
    pub id: u64,
    pub name: String,     // courseName + courseInfoName(如 羽毛球)
    pub time: String,     // classTime,如 周一34节
    pub teacher: String,
    pub loc: String,
    pub used: u64,        // totalReservedNumber
    pub cap: u64,         // totalStudents
    pub sex: String,      // 男/女/混/比
}

impl SportClass {
    pub fn label(&self) -> String {
        format!("{} {} {} {}", self.name, self.time, self.teacher, self.loc)
    }
}

/// 一个体育账号的抢课配置(持久化到 cfg:name/token/ids_str/note)
#[derive(Clone, Serialize, Deserialize)]
pub struct SportAcc {
    pub name: String,
    pub token: String,
    pub ids_str: String,  // 逗号分隔的志愿班级 id 序列,从上到下依次抢
    pub note: String,
    #[serde(skip)]
    pub st: String,       // idle/armed/run/ok/fail/err(仅运行时)
    #[serde(skip)]
    pub sub: String,      // 副行(最近状态文字,仅运行时)
}

impl SportAcc {
    pub fn new(name: &str, token: &str, ids: &[u64], note: &str) -> Self {
        SportAcc {
            name: name.into(),
            token: token.into(),
            ids_str: ids.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(","),
            note: note.into(),
            st: "idle".into(),
            sub: String::new(),
        }
    }
    pub fn parsed_ids(&self) -> Vec<u64> {
        self.ids_str.split([',', '，', ' ', ';'])
            .filter_map(|s| s.trim().parse::<u64>().ok())
            .collect()
    }
}

/// 体育账号运行状态 -> 展示文字/颜色样式 key
impl SportAcc {
    pub fn st_text(&self) -> &'static str {
        match self.st.as_str() {
            "armed" => "已盯防",
            "run" => "抢课中",
            "ok" => "已入册",
            "fail" => "停止",
            "err" => "token无效",
            _ => "待命",
        }
    }
    pub fn st_cls(&self) -> &'static str {
        match self.st.as_str() {
            "armed" => "arm",
            "run" => "run",
            "ok" => "ok",
            "fail" => "fail",
            "err" => "err",
            _ => "",
        }
    }
}

impl Default for AppData {
    fn default() -> Self {
        AppData {
            base: DEFAULT_BASE.to_string(),
            profile: DEFAULT_PROFILE.to_string(),
            cookie: String::new(),
            cookie_visible: false,
            account: String::new(),
            password: String::new(),
            session_msg: String::new(),
            elections: Vec::new(),
            keyword: String::new(),
            quick_kw: String::new(),
            courses: Vec::new(),
            login_state: "空闲".to_string(),
            login_open: false,
            auto_fetch_after_login: true,
            pending_open_login: false,
            targets: Vec::new(),
            strategy: Strategy::Free,
            refresh_ms: 1500,
            submit_ms: 400,
            auto_stop_done: true,
            one_per_no: true,
            running: false,
            engine_label: "空闲".to_string(),
            rounds: 0,
            started_at: None,
            submit_path: "stdElectCourse!batchOperator.action".to_string(),
            submit_query: "profileId={pid}&retakeDetail=".to_string(),
            submit_body: "optype=true&operator0={id}:true&lesson0={id}&schLessonGroup_{id}=0&retakeDetail=".to_string(),
            count_extra: String::new(),
            semester_id: String::new(), // 首次拉课时自动从选课页提取
            data_path: "stdElectCourse!data.action".to_string(),
            sport_lib: Vec::new(),
            sport_accounts: default_sport_accounts(),
            sport_fire: "21:00".to_string(),
            sport_interval_ms: 600,
            sport_gate_fire: false,
            sport_auto_arm: true, // 默认启动程序即自动盯防体育
            sport_active: false,
            sport_msg: "空闲".to_string(),
            sport_kw: String::new(),
            logs: VecDeque::new(),
            last_raw: String::new(),
            tab: Tab::Session,
            curl_input: String::new(),
            cfg_path: String::new(),
        }
    }
}

fn default_sport_accounts() -> Vec<SportAcc> {
    // 占位示例:token 与志愿班级请在「体育抢课」页填写(cfg.json 持久化,不会提交到仓库)。
    vec![
        SportAcc::new("示例账号1", "", &[], "在此填入你的 ACCESS_TOKEN 与志愿班级 id"),
        SportAcc::new("示例账号2", "", &[], "可删除或继续添加"),
    ]
}

impl AppData {
    pub fn log(&mut self, lvl: Lvl, msg: impl Into<String>) {
        let t = chrono::Local::now().format("%H:%M:%S").to_string();
        self.logs.push_back(LogLine { t, lvl, msg: msg.into() });
        while self.logs.len() > 900 { self.logs.pop_front(); }
    }
}
