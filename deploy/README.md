# 部署（本机服务器）

目标形态（用户 2026-11-18 定稿）：

- **主机**：这台 Mac（公司常开），`launchd` 守护 `center-server`，只监听 `127.0.0.1:8787`
- **入口**：`tailscale serve --bg --https=443 http://127.0.0.1:8787` → 手机/另一台电脑打开 `https://<机器名>.<tailnet>.ts.net`
- **冷备**：i3 PC（同 tailnet）跑 Docker/WSL2，从 WebDAV 备份恢复，目标 ≤15 分钟接管（M5 演练）
- **不买云服务器**：成本只有云端 OCR（约 0.003 元/张）

## 安装 / 更新

```bash
npm run build                       # 前端产物给服务端托管
bash deploy/install-macos.sh        # 构建 release + 写 launchd + 健康检查 + 挂 tailscale serve
```

更新流程同上（`launchctl bootout` 由脚本内部做，服务会重启一次，数据不动）。

## 常用运维

```bash
launchctl print gui/$UID/com.center.server | head -20     # 状态
tail -f "$HOME/Library/Application Support/VoucherStudio/logs/center-server.err.log"
tailscale serve status                                    # 私网入口
launchctl bootout gui/$UID/com.center.server              # 停
bash deploy/install-macos.sh --uninstall                  # 卸载（数据保留）
```

## 必须知道的三件事

1. **合盖会睡**：`sudo pmset -a disablesleep 1`（建议接电源常开；拔电前 `disablesleep 0`）。
2. **FileVault**：重启后要有人在本机登录一次，服务才能起来；长期断电重启后先远程确认。
3. **数据目录**：`CENTER_DATA_DIR`（默认 `~/Library/Application Support/VoucherStudio`）。备份靠应用内的 WebDAV 上传，建议每天一次。

## 安全

- 只监听回环地址，公网不可达；外部访问必须经过 Tailscale（或你自己的反代 + TLS）。
- 访问密码 PBKDF2-SHA256（12 万轮）存在 `auth_users`；会话 Cookie `HttpOnly; SameSite=Lax`。
- 服务端密钥（云端 OCR Key、WebDAV 密码、备份密码）存 `config/secrets.json`（0600），不再走 macOS Keychain——无 GUI 的后台进程读 Keychain 会阻塞。
