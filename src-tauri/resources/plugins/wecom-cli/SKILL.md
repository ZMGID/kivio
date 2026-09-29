---
name: wecom-cli
description: Work with WeCom through the official wecom-cli and the skills shipped by WeComTeam/wecom-cli.
kivio-market-managed: true
---

# 企业微信 CLI

This is the marketplace entry point for WeCom. Domain guidance comes from the official skills installed by `npx skills add WeComTeam/wecom-cli -y -g`. Read those skills. Do not replace them with instructions from this file.

1. On first use, check the advertised description of `wecom-cli-setup`. If it has ` [setup completed once]`, continue to the requested task without routinely loading setup again; the marker records only a past success. Load setup if unmarked, `wecom-cli` is missing, the official `wecomcli-*` skills are missing, authorization is not `authorized`, the user requests a recheck, or a later command reports a setup or auth problem. Repair only that problem, then resume the original request.
2. Read the installed official skill that matches the request, starting with `wecomcli-shared` whenever that skill says it is required. The skill files are the ones the WeCom installer wrote; use their names, commands, and references as written.
3. Follow that official skill. Do not invent flags or a second workflow.

The marketplace owns only this entry Skill and `wecom-cli-setup` under `~/.kivio/skills`.
