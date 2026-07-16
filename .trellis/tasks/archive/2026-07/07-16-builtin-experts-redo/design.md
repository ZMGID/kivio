# Design — 重做内置专家套件

## 〇、共享文风块（去 AI 味 · 每个 prompt 都嵌入，R6）
以下作为每个 system_prompt 结尾的固定段（写作/翻译/文档专家再加领域强化）：

> 写作要求（务必遵守）：像具体的人写的，不是"AI 生成"的。
> - 直给结论和内容，不复述我的问题，不写"当然/好的/很高兴为你"这类开场。
> - 不用套话与空转过渡（"综上所述""总而言之""在当今…的时代""值得注意的是""首先/其次/再次"式凑数）。
> - 不无脑分点、不无脑加粗、不滥用 emoji；能用连贯段落就别拆成清单，清单只在真正并列时用。
> - 不堆形容词、不拔高升华、不写正确的废话；每句话要有信息量。
> - 不过度免责和模棱两可（少用"可能也许某种程度上或许"），有判断就直说，不确定就点明哪里不确定。
> - 中文就写地道中文，别带翻译腔和英式长句；句子长短交错，像正常人说话。
> - 默认用与用户相同的语言。

实现上抽成一个 Rust 常量 `NO_AI_FLAVOR_STYLE`，各专家 prompt 末尾拼接，避免重复且统一维护。

## 一、专家名册提案（7 个，评审确认）
留空 `provider_id`/`model`（跟随用户默认）；中文；system_prompt 统一含【角色/擅长】【工作方式】【输出规范】【边界与诚实性】【共享文风块】。skill_ids 只挂相关且非连接器门控的内置技能。

| id | 名称 | 图标 | 领域定位（与他人不重叠） | skill_ids |
|---|---|---|---|---|
| `asst_builtin_writer` | 写作助手 | ✍️ | 中短篇：文章/邮件/文案/演讲稿的起草·改写·润色·精简，控读者与语气 | doc-coauthoring, docx, pdf |
| `asst_builtin_coder` | 编程助手 | 💻 | 通用软件工程：读写/调试/重构/解释，最小聚焦改动 | diagram |
| `asst_builtin_frontend` | 前端设计师 | 🎨 | UI/前端：设计品味 + 生产级实现（区别于 coder 的工程向） | frontend-design, diagram |
| `asst_builtin_researcher` | 研究助手 | 🔍 | 联网检索 + 交叉核实 + 带出处综述（只读不改文件） | diagram |
| `asst_builtin_data` | 数据分析 | 📊 | 读 PDF/Excel/Word，用 Python 沙箱清洗·统计·可视化 | pdf, docx, xlsx, diagram |
| `asst_builtin_translator` | 翻译助手 | 🌐 | 翻译/本地化：术语一致、语气还原，可译附件文档 | docx, pdf |
| `asst_builtin_docsmith` | 文档专家 | 📄 | 长篇结构化交付物：报告/PRD/规格/方案，多节 + 表格 + 图 | doc-coauthoring, docx, xlsx, pdf, diagram |

差异化要点：写作(中短散文/语气) vs 文档专家(长篇多节结构化) vs 前端设计师(UI 设计+实现) vs 编程(通用工程)。可选第 8 个（图表可视化）暂不加，避免与 data/diagram 重叠。

## 二、后端改动
### 定义（`chat/storage.rs::builtin_assistant_definitions`）
重写 4 个 + 新增 3 个（frontend/translator/docsmith），共 7 条，system_prompt 专业化。

### 非破坏性 v2 迁移（核心）
- `settings.rs`：新增 `builtin_assistants_seeded_v2: bool`（default false），紧邻 v1（字段 `:1173`/默认 `:1318`）。
- `chat/storage.rs`：新增 `merge_builtin_assistants_v2(app, now) -> Result<(),String>`：
  1. `load` 现有 index（无则空）。
  2. 以 `builtin_assistant_definitions(now)` 为准，**按 id upsert**：同 id 项替换、缺失则追加。
  3. **保留所有其它条目**（用户自建/非本次内置 id 的项）。
  4. `save_assistant_index`。
  - 权衡：若用户曾编辑过某内置专家（同 id），会被新版覆盖——"重做内置"语义下可接受，PRD Out-of-scope 记明。
- `lib.rs`（v1 块 `:243` 之后）：加 v2 守卫块——`if !settings.builtin_assistants_seeded_v2 { merge_builtin_assistants_v2(...)?; 置 flag; persist（失败回滚重试）}`。
  - 新装：v1 先 seed 全新 7 条 → v2 merge 为幂等 no-op → 两 flag 均 true。
  - 老装(已 v1)：v1 跳过 → v2 merge upsert 新/改内置、保留用户自建。
- 幂等：v2 flag 置 true 后不再跑（AC3）。

## 三、前端改动
- `builtinAssistantGlyph`（定义处：`src/chat/assistantIcons.tsx` 或 `AssistantCenter.tsx:402` 引用）补 3 个新 id → glyph（🎨/🌐/📄）。找不到映射则回退首字母，故为兼容也应补。
- 无需改 `ChatAssistant` TS 结构（不加字段）。settings 的 v2 flag 为后端启动用，若 AppSettings TS 未镜像 v1 flag 则 v2 也无需镜像。

## 四、契约 / 兼容
- 不改 `ChatAssistant`/`ChatAssistantSnapshot` 结构 → 无快照兼容问题。
- 仅 `AppSettings` 加一个 bool（serde default=false）→ 老 settings.json 反序列化安全。

## 五、风险 / 回滚
- 风险：v2 merge 覆盖用户已编辑的同 id 内置项（已记 out-of-scope）；merge 逻辑是新代码需单测。
- 回滚：定义重写、v2 迁移、前端图标三块相对独立；迁移出错时 flag 不置位、下次重试，不破坏既有数据（merge 保留用户项）。
- 测试：`storage.rs` 现有 `builtin_assistant_tests`（`:1522`，断言"4 个"）需更新为新数量；新增 merge upsert/保留用户项的单测。
</content>
