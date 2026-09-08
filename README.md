English | [中文](README.zh-CN.md)
# URP/eams Course Grabber (Desktop)

Automated course-selection helper for URP/eams (Zhengfang-style) academic systems, with an extra multi-account tab for "smart sport" (physical-education) H5 election portals.

> No credentials are ever stored — the app reuses **your own** authenticated session (session Cookie / Bearer token) and automates the clicks you would otherwise do by hand.

## Features

- **One-click login**: a built-in WebView2 window completes the SSO flow; the session cookie is auto-captured and the course list auto-fetched — no F12 / copy-paste
- **Keyword search & quick grab**: search by course name / course code / lesson ID, then add as a target or grab instantly
- **Dual modes**:
  - ⚡ **Quick**: submit continuously regardless of quota — for the moment the window opens
  - 🕐 **Watch**: submit only when a seat frees up, or monitor-only
- **Adjustable intervals** (refresh & submit), one-click presets, live per-attempt feedback with a full log
- **Conflict-safe**: once one lesson of a course number is grabbed, sibling targets auto-disable
- **Sport tab (multi-account)**: fill each account's ACCESS_TOKEN + priority lesson IDs; the engine checks enrollment every 8s, re-grabs instantly if the system clears a record, and sends a confirmation at the configured opening time
- **Auto-arm on launch** (toggleable) and **token hot-reload** — update a token in `cfg.json` and it takes effect without restarting

## Build

- Rust stable toolchain; Windows 10/11 with the **WebView2 runtime** (usually preinstalled with Edge)
- `cargo build --release` — or double-click `build.bat`
- Output: `target\release\chd-course-grabber.exe`

## Usage

1. Open the app → **Session** tab → **Open built-in browser login**
2. Finish authentication in the popup window; the cookie is saved and courses are loaded automatically
3. **Course Search**: type a keyword → **Quick Grab**
4. **Grab Center**: pick a mode and interval → **Start**
5. (Optional) **Sport** tab: add accounts (token + priority lesson IDs) → **Start engine**

Config (including the session cookie) is stored in `cfg.json` next to the executable — never share or commit it. Passwords are never written to disk.

## How It Works (APIs Used)

| Endpoint | Purpose |
|---|---|
| `GET /eams/stdElectCourse!data.action?profileId=<id>` | Course list (lesson IDs, course codes) |
| `GET /eams/stdElectCourse!queryStdCount.action?projectId=1&semesterId=<id>` | Selected/capacity count per lesson |
| `POST /eams/stdElectCourse!batchOperator.action` | Submit course election |
| `GET <sport-gw>/student-course-arrange-page` | Sport class list & quota |
| `GET <sport-gw>/student-course-scores` | Enrolled-record check (the "you're in" endpoint) |
| `POST <sport-gw>/reserve?arrangeId=<id>` | Reserve a sport class |

Standard action templates are built in. If a deployment changed an action, paste a real request as cURL into the **Advanced** tab to recalibrate — no code change needed.

## Notes

- Use only with your **own** account(s) and only for courses you actually need
- Keep intervals conservative; high request frequency may trigger school anti-abuse protection
- Use only during officially announced election windows and follow your school's rules

## License

This project is licensed under a custom personal-use license. See the [LICENSE](LICENSE) file for full terms.

**TL;DR:** Personal learning, research, and self-deployment with your own account only. Commercial use, third-party services, and redistribution are prohibited. Use at your own risk.
