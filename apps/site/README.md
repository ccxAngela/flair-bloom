# 产品页 app.xwink.fun/flair-bloom

气质花的对外门面：下载安装包、看更新公告、找支持入口。纯静态 SSG，挂在 app.xwink.fun 的路径下；域名、证书与 `app.xwink.fun` 的 server 块归 xwink-console 仓的 app-site 单元，本目录只负责自己这一段。

它同时是 `@xwink/*` 私有制品的仓外消费者：按精确版本从私有 registry 安装，不引用 xwink-console 源码。

「支持&帮助」提供产品 QQ 群 `1127449168` 与复制按钮，首页下载区及自动下载页提供站内引导；群号直接复用 `apps/main/src/constants.ts` 的 `PRODUCT_QQ_GROUP`，与应用「关于」保持一致。未提供加群链接时只复制群号，由用户在 QQ 中搜索。打印物料不加入群号或引流入口。

## 组成

| 目录      | 作用                                                                                       |
| --------- | ------------------------------------------------------------------------------------------ |
| `app/`    | Nuxt 页面。公告 Markdown 解析与门派色板直接引用 `apps/main` 的同一份源码，网站与应用不会漂 |
| `deploy/` | 部署脚本、nginx 片段、服务器主机公钥基线                                                   |

为什么不并入仓库根 workspace：根 workspace 的 Tauri 发版流水线会跑 `pnpm install`，并进来就得给那条流水线也配私有制品库凭据。

## 版本数据与下载

页面在浏览器里读 GitHub Releases API（`gh-proxy.com` 代理优先、直连兜底），取最近 10 个稳定版本渲染公告与下载清单；下载地址是加速代理后的 GitHub 地址。安装包流量不经本站服务器，服务器只托管静态页面。

发版后产品页自动跟上，不用重发站点。两条链路都读不到时页面退回「前往 GitHub 发布页」。应用内更新器走的是同一套加速策略，但端点是 GitHub Releases 的 `latest.json`，与本页无关，见主仓 skill 的 `updater.md`。

## 分享下载链接

`https://app.xwink.fun/flair-bloom/download` 打开即开始下载最新版的 exe，加 `?type=msi` 下载 MSI。地址固定，指向哪个版本由页面当场从 GitHub 读出来。

## 打印海报与说明书

在产品页直接 Cmd+P / Ctrl+P，打印成两张 A4：第一张是宣发海报，第二张是使用说明，配色跟当前选的门派色走。纸张选 A4、勾选「背景图形」，页眉页脚不用手动关。

物料会被转发到论坛、群聊，那边限制站外链接与二维码，所以不放二维码、不做引流，每页底部只留一行纯文本下载地址，按打印时所在站点现算。海报的骚话只取 `rumors` 前四条作者写的，不带玩家评论。打印排版在 `app/components/PrintKit.vue`，文案与页面共用 `app/utils/content.ts`，改一处两边一起变。

## 本地开发

```sh
cd apps/site
# 私有制品库凭据写用户级 ~/.npmrc（pnpm 11 不展开项目级 .npmrc 里的环境变量）
pnpm config set --location=user //npm.cnb.cool/x-wink/playground/npm/-/packages/:_authToken <令牌>
pnpm install
pnpm dev          # http://localhost:3900/flair-bloom/
pnpm typecheck && pnpm build
```

## 发布

推 `site-v<semver>` tag 触发 `.github/workflows/site.yml`：类型检查、构建、上传、原子切换版本、验收。产品页发版不经 Tauri updater，不触达应用用户。回滚在服务器上把 `/opt/flair-bloom/www/flair-bloom` 指回 `/opt/flair-bloom/site/previous` 所指目录。

## 服务器开通（一次性，需要 root）

前提：xwink-console 的 app-site 单元已上线，`/etc/nginx/vhost.d/app.xwink.fun/` 存在。

1. 建部署用户与目录。CI 用这个用户上传与切换版本，它没有 sudo：

   ```sh
   useradd --system --create-home --shell /bin/bash flair-bloom
   install -d -o flair-bloom -g flair-bloom /opt/flair-bloom
   # 把 CI 部署公钥写进 /home/flair-bloom/.ssh/authorized_keys
   ```

2. 守护式下发 nginx 片段。它几乎不变，不交给 CI：能改它的密钥就等同 root。

   ```sh
   winkops edit /etc/nginx/vhost.d/app.xwink.fun/flair-bloom.conf --file apps/site/deploy/flair-bloom.conf \
     --validate 'nginx -t' --reload 'systemctl reload nginx' -c <连接配置>
   ```

3. GitHub 配置 secrets，分两级：
   - `production` 环境，部署来源只允许 `site-v*` tag：`SITE_DEPLOY_HOST`、`SITE_DEPLOY_PORT`、`SITE_DEPLOY_USER`（`flair-bloom`）、`SITE_DEPLOY_SSH_KEY`。私钥只存在这里，公钥在服务器 `authorized_keys`，本地不留副本。
   - 仓库级：`XWINK_NPM_TOKEN`，私有制品库只读令牌。构建任务不绑定环境，放进环境就读不到。

4. 推第一个 `site-v*` tag。

已经跑过发布镜像的机器还要收尾一次：`systemctl disable --now flair-bloom-mirror.timer flair-bloom-mirror.service`，删掉 `/etc/systemd/system/flair-bloom-mirror.*` 与 `/opt/flair-bloom/mirror`、`/opt/flair-bloom/mirror-app`。
