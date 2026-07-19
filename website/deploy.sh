#!/bin/bash
# 部署 website/ 到 GreenCloudVPS 上的 Caddy 静态站点。详见 DEPLOY.md。
set -euo pipefail

HOST="${KIVIO_DEPLOY_HOST:-root@185.200.65.236}"
DEST="${KIVIO_DEPLOY_DEST:-/opt/domain-gateway/sites/kivio-desktop}"
SRC="$(cd "$(dirname "$0")" && pwd)"

# 上传（排除文档/脚本，不发到公开目录）
COPYFILE_DISABLE=1 tar czf - -C "$SRC" \
  --exclude=deploy.sh --exclude=DEPLOY.md --exclude=STYLE.md . \
  | ssh -o BatchMode=yes -o StrictHostKeyChecking=accept-new "$HOST" \
      "tar xzf - -C '$DEST' && echo '已更新：'$DEST"

echo "完成 → https://kivio-desktop.xyz"
