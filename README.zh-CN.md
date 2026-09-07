English | [中文](README.zh-CN.md)
# URP/eams 选课助手 (桌面版)

URP/eams(正方同系)教务系统自动选课桌面工具,另附一个面向"智慧体育"H5 选课门户的多账号抢课页签。

> 绝不保存任何账号凭据 —— 程序复用**你自己**已认证的会话(Cookie / Bearer token),把你在浏览器里手工反复点击的"选课"自动化。

## 功能

- **一键登录**:内置 WebView2 窗口完成统一认证;会话 Cookie **自动抓取**、课程列表 **自动拉取**,无需 F12 / 复制粘贴
- **关键词检索 & 快捷抢课**:按课程名 / 课程代码 / 教学班ID 搜索,一键加入目标或直接开抢
- **双模式**:
  - ⚡ **快速**:无视余量持续提交,适合选课窗口刚开放的瞬间
  - 🕐 **蹲守**:有余位才提交,或仅监控
- **间隔可调**(刷新 / 提交两档)、一键预设、每次提交都有实时反馈与完整日志
- **防冲突**:同一课程序号抢到一门后,其余教学班自动停用
- **体育页签(多账号)**:每个账号填 ACCESS_TOKEN + 志愿教学班ID;引擎每 8 秒核对入册记录,记录被系统清掉立即抢回,到设定开抢时刻自动发一次通道确认
- **启动即自动布防**(可关)与 **token 热更新**:外部改写 cfg.json 后无需重启即时生效

## 构建

- Rust stable 工具链;Windows 10/11 需 **WebView2 运行时**(通常随 Edge 已装)
- `cargo build --release` —— 或双击 `build.bat`
- 产物:`target\release\chd-course-grabber.exe`

## 使用

1. 打开程序 → **会话登录** 页 → 点 **打开内置浏览器登录**
2. 在弹出的窗口完成认证;Cookie 自动保存、课程自动载入
3. **课程检索**:输入关键词 → **快捷自动抢**
4. **抢课中心**:选模式与间隔 → **▶ 开始**
5. (可选)**体育抢课** 页:添加账号(token + 志愿教学班ID)→ **启动引擎**

配置(含会话 Cookie)保存在 exe 同目录 `cfg.json`,**切勿外传或提交**;密码永不落盘。

## 内部使用的接口

| 接口 | 用途 |
|---|---|
| `GET /eams/stdElectCourse!data.action?profileId=<id>` | 课程列表(教学班ID、课程代码) |
| `GET /eams/stdElectCourse!queryStdCount.action?projectId=1&semesterId=<id>` | 每个教学班已选/上限人数 |
| `POST /eams/stdElectCourse!batchOperator.action` | 提交选课 |
| `GET <sport网关>/student-course-arrange-page` | 体育教学班列表与余量 |
| `GET <sport网关>/student-course-scores` | 入册记录核对("你选上了"的铁证接口) |
| `POST <sport网关>/reserve?arrangeId=<id>` | 预约体育教学班 |

标准 action 模板已内置。若某校改版导致拉不到/提交失败,把真实请求以 cURL 形式粘贴进 **高级 · 模板** 页校准即可,无需改代码。

## 注意事项

- 只在**你自己**的账号上使用,只抢自己真正需要的课
- 保持合理请求间隔;频率过高可能触发学校风控
- 仅在教务处公布的选课时间内使用,并遵守你所在学校的规定

## License

仅供个人学习使用。请合理使用,风险自负。
