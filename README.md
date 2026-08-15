# AI作战室

> 多模型AI网关桌面端 — 一键安装，本地运行，支持所有主流AI模型

## 这是什么

一个打包了 [One API](https://github.com/songquanpeng/one-api)（MIT协议）的桌面应用。安装后双击运行，自动启动本地AI网关，通过浏览器界面管理你的API Key、模型渠道和令牌。

**自用 + 出售双用途**：自己用是本地AI网关，改Logo/名称后可作为白标产品出售。

## 技术栈

| 层 | 技术 | 说明 |
|---|---|---|
| 桌面壳 | Tauri 2 (Rust) | 安装包~10MB，比Electron轻10倍 |
| 网关引擎 | One API (Go) | 65MB单文件，MIT协议，SQLite零配置 |
| 前端 | 原生HTML/CSS/JS | 启动动画+iframe加载One API原生界面 |

## 项目结构

```
project/
├── package.json              # 前端依赖
├── index.html                # 启动页（splash→iframe）
├── vite.config.js            # Vite配置
├── build.bat                 # Windows一键构建脚本
├── .github/workflows/
│   └── build.yml             # GitHub Actions云端构建
├── README.md
└── src-tauri/
    ├── Cargo.toml            # Rust依赖
    ├── tauri.conf.json       # Tauri配置
    ├── build.rs
    ├── icons/                # 应用图标
    ├── binaries/             # One API二进制（构建时下载）
    ├── capabilities/
    │   └── default.json      # 权限配置
    └── src/
        ├── main.rs           # Rust入口
        └── lib.rs            # 核心逻辑（sidecar管理）
```

## 工作原理

1. 用户双击启动"AI作战室"
2. Tauri主进程启动，显示splash启动画面
3. Rust调用`start_gateway`命令，以sidecar方式启动One API
4. One API在随机可用端口运行，数据存在`%APPDATA%/com.aiwarroom.desktop/gateway/`
5. 检测到One API就绪后，WebView通过iframe加载`http://127.0.0.1:PORT`
6. 用户看到One API原生Web界面，登录root/123456开始配置
7. 关闭窗口时One API自动终止

## 构建方式

### 方式一：本地构建（需要Node.js + Rust）

```batch
:: 双击运行 build.bat
:: 或手动执行：
npm install
npm run tauri build
```

产物：
- MSI安装包：`src-tauri/target/release/bundle/msi/`
- NSIS安装包：`src-tauri/target/release/bundle/nsis/`

### 方式二：GitHub Actions云端构建（推荐，不需要本地环境）

1. 将项目推送到GitHub仓库
2. 打tag：`git tag v0.1.0 && git push --tags`
3. Actions自动构建，在Releases页面下载MSI

## 白标定制

出售前修改以下内容：

1. **应用名称**：`src-tauri/tauri.conf.json` → `productName`
2. **窗口标题**：`index.html` → `<title>` 和 `.app-title`
3. **图标**：替换 `src-tauri/icons/` 下所有文件
4. **标识符**：`tauri.conf.json` → `identifier`（改为你的域名）
5. **One API品牌**：One API本身是MIT协议，界面中"One API"字样可通过自定义前端构建去除

## 开发计划

- [x] 项目骨架 + sidecar启动逻辑
- [x] 应用图标
- [x] GitHub Actions自动构建
- [x] 随机端口 + 数据目录隔离
- [ ] 首次启动引导（修改默认密码、填API Key）
- [ ] 系统托盘 + 最小化到托盘
- [ ] 开机自启选项
- [ ] 中文UI定制
- [ ] 白标版本（不同品牌打包）
- [ ] macOS构建支持
- [ ] 自动更新

## 许可证

- AI作战室外壳：MIT
- One API底座：MIT（https://github.com/songquanpeng/one-api）
