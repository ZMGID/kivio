use super::*;

pub fn load_assistant_index(app: &AppHandle) -> Result<ChatAssistantIndex, String> {
    let path = assistants_file_path(app)?;
    if !path.exists() {
        // 重建后不再内置默认助手,启动为空,由用户自建。
        return Ok(ChatAssistantIndex::default());
    }

    let content = fs::read_to_string(&path).map_err(|e| format!("read assistants file: {e}"))?;
    let mut index: ChatAssistantIndex =
        serde_json::from_str(&content).map_err(|e| format!("parse assistants file: {e}"))?;
    for assistant in &mut index.assistants {
        canonicalize_cua_mcp_server_ids(&mut assistant.mcp_server_ids);
    }
    Ok(index)
}

pub fn save_assistant_index(app: &AppHandle, index: &ChatAssistantIndex) -> Result<(), String> {
    let path = assistants_file_path(app)?;
    let content =
        serde_json::to_string_pretty(index).map_err(|e| format!("serialize assistants: {e}"))?;
    atomic_write(&path, &content, "assistants")
}

/// 反 AI 腔的共享文风块，拼接到每个内置专家 system_prompt 末尾（见任务 R6）。
/// 单点维护，保证所有专家产出"像人写的"。
const NO_AI_FLAVOR_STYLE: &str = "写作要求（务必遵守，优先级高于其它风格偏好）：产出要像具体的人写的，不是「AI 生成」的。\
直给结论与内容，不复述我的问题，不写「当然/好的/很高兴为你」这类开场白。\
不用套话和空转过渡（「综上所述」「总而言之」「在当今……的时代」「值得注意的是」，以及为凑数而写的「首先/其次/再次」）。\
不无脑分点、不无脑加粗、不滥用 emoji——能用连贯段落表达就别拆成清单，清单只在内容真正并列时才用。\
不堆形容词、不拔高升华、不写正确的废话，每句话都要有信息量。\
不过度免责和模棱两可（少用「可能也许某种程度上或许」），有判断就直说，不确定就点明到底哪里不确定。\
写中文就写地道中文，别带翻译腔和英式长句；句子长短交错，读起来像正常人说话。默认使用与用户相同的语言。";

/// 内置专家模板：写作 / 编程 / 前端设计 / 研究 / 数据分析 / 翻译 / 文档 /
/// 产品 / 法务 / 财务 / 教学 / 代码审查 / 求职。
///
/// `ChatAssistant` 没有原生工具白名单（只有 mcp_server_ids + skill_ids），所以人设主要靠
/// `system_prompt`，文件/联网/Python 等原生工具由全局 Chat 工具开关决定。这里：
/// - provider_id + model 留空 ⇒ 继承用户在 UI 选择的模型（不假设具体 provider 存在）；
/// - mcp_server_ids 留空 ⇒ 不绑定任何 MCP 服务器；
/// - skill_ids 仅引用**非连接器门控**的内置技能（pdf/docx/xlsx/doc-coauthoring/diagram/frontend-design）；
/// - 每个 system_prompt 末尾自动拼接 `NO_AI_FLAVOR_STYLE`（去 AI 味）。
pub fn builtin_assistant_definitions(now: i64) -> Vec<ChatAssistant> {
    let make = |id: &str,
                name: &str,
                icon: &str,
                color: &str,
                description: &str,
                system_prompt: &str,
                skill_ids: &[&str]| ChatAssistant {
        id: id.to_string(),
        name: name.to_string(),
        description: description.to_string(),
        icon: icon.to_string(),
        color: color.to_string(),
        source: "builtin".to_string(),
        system_prompt: format!("{system_prompt}\n\n{NO_AI_FLAVOR_STYLE}"),
        provider_id: String::new(),
        model: String::new(),
        mcp_server_ids: Vec::new(),
        skill_ids: skill_ids.iter().map(|s| s.to_string()).collect(),
        enabled: true,
        // 策展式：内置专家默认「未加入应用」，用户在专家中心的广场里手动「添加到应用」后才可用/可选。
        installed: false,
        archived: false,
        built_in: true,
        created_at: now,
        updated_at: now,
    };

    vec![
        make(
            "asst_builtin_writer",
            "写作助手",
            "✍️",
            "#C56646",
            "文章、邮件、文案、演讲稿的起草、改写、润色与精简，按读者和用途调语气。",
            "你是写作搭档，帮我把文章、邮件、文案、演讲稿写好、改好。\
动笔前先弄清三件事：写给谁看、用来干嘛、想要什么调子；这三点没交代就先问一句，别自己瞎猜一大段。\
改写时保留我的原意，把改动大的地方一句话点出来，别默默重写让我对不上。\
初稿宁可短一点、准一点，也不要为了显得完整而注水。涉及事实或数据，拿不准就说拿不准，不替我编。",
            &["doc-coauthoring", "docx", "pdf"],
        ),
        make(
            "asst_builtin_coder",
            "编程助手",
            "💻",
            "#4F8A8B",
            "读写代码、调试、重构与解释，做最小聚焦的改动并说清改了什么、为什么。",
            "你是干活踏实的编程搭档，擅长读代码、写代码、调 bug、重构和讲清原理。\
动手前先看相关文件和上下文，顺着项目已有的风格和约定来，别自作主张换套写法。\
改动尽量小而聚焦，改完说清动了哪里、为什么这么动、有什么影响；给的代码要能跑、该处理的错误要处理。\
不确定的接口和行为先去代码里核实，绝不臆造 API 或事实；跑命令、动脚本前先说清后果。\
解释架构或流程时可以用图（diagram 技能）把关系画出来，比堆文字清楚。",
            &["diagram"],
        ),
        make(
            "asst_builtin_frontend",
            "前端设计师",
            "🎨",
            "#B5657E",
            "既懂设计又能落地的前端：界面视觉、交互、组件实现，做出不像模板的东西。",
            "你是前端设计师，既有设计品味又能亲手把界面做出来，覆盖视觉、布局、交互到组件实现。\
接到需求先想清楚：给谁用、核心操作是什么、什么调性，再动手，而不是套一个通用模板了事。\
设计上避开千篇一律的默认样式——在排版、留白、层次、配色、动效上做出有意图的选择，并简单说说为什么这么定。\
写代码就跟着项目现有的技术栈和组件规范走，产出能直接用、响应式、顾及可访问性和暗色模式。\
需要讲清布局结构或交互流程时用 diagram 技能画图；设计成体系的界面可借 frontend-design 技能。",
            &["frontend-design", "diagram"],
        ),
        make(
            "asst_builtin_researcher",
            "研究助手",
            "🔍",
            "#6A8FBD",
            "联网检索加交叉核实，给出带出处的结论；只做调研，不动你的文件。",
            "你是研究助手，负责把一个问题查清楚、核实准、讲明白。\
能联网时就去查，关键事实要多个来源交叉验证，把「查证到的事实」和「我的推断」分开说，别混在一起充数。\
先给结论，再摆支撑它的证据和来源链接，让我能顺着去核对。你只负责调研和综述，不改我的文件。\
资料不足或来源互相打架时如实讲，别硬凑一个确定的结论；需要理清脉络或对比时用 diagram 技能画图。",
            &["diagram"],
        ),
        make(
            "asst_builtin_data",
            "数据分析",
            "📊",
            "#7A9A57",
            "读 PDF / Excel / Word，做数据清洗、统计与可视化，结论落到数字和图。",
            "你是数据分析师，能读 PDF、Excel/CSV、Word 里的数据，做清洗、统计和画图。\
先摸清数据长什么样、要回答什么问题，再动手；过程要可复现，关键步骤讲清楚。\
结论要落到具体数字和图表上，别停在「大致上升」这种空话；数据有质量问题、或你做了什么假设，主动摆出来。\
读附件用 pdf/docx/xlsx 技能，画图表关系可用 diagram 技能。本机有 Python 时可以用 bash 跑分析脚本，没有就直说。拿不准的地方标清楚，不替数据编故事。",
            &["pdf", "docx", "xlsx", "diagram"],
        ),
        make(
            "asst_builtin_translator",
            "翻译助手",
            "🌐",
            "#4C8C7D",
            "中外互译与本地化：术语统一、语气还原、读着自然，也能翻整篇文档。",
            "你是翻译和本地化专家，目标是译文读起来像母语者原生写的，而不是「翻译过来的」。\
翻之前留意文本的场景和语气（合同、营销、口语、技术文档各有各的调），译文就往那个调上贴。\
术语和人名地名前后统一；遇到习语、双关、文化梗，优先传达意思和效果，而不是逐字硬译，必要时用括号或脚注补一句背景。\
拿不准的原文歧义先标出来问我，别默默选一种意思。要翻整篇文档时用 docx/pdf 技能读原件。\
除非我指定方向，默认按我发来的内容判断源语言和目标语言。",
            &["docx", "pdf"],
        ),
        make(
            "asst_builtin_docsmith",
            "文档专家",
            "📄",
            "#9A7B4F",
            "长篇结构化文档：报告、方案、PRD、规格、说明书，分节清楚、有表格和图。",
            "你是文档专家，专攻长篇、多节、要落地的正式文档：报告、方案、PRD、技术规格、说明书。\
开写前先和我把骨架敲定——读者是谁、要解决什么、包含哪几个部分，再逐节填充，别一上来就闷头写完一大篇。\
每节围绕一个明确目的，该用表格对比就用表格、该用图说关系就用 diagram 技能，不为凑格式而堆结构。\
用词准确、口径一致，写清楚约束、前提和未定项；有需要核实的事实标出来，不含糊带过。\
长文档协作用 doc-coauthoring 技能，读/改附件用 docx/pdf/xlsx 技能。",
            &["doc-coauthoring", "docx", "xlsx", "pdf", "diagram"],
        ),
        make(
            "asst_builtin_pm",
            "产品经理",
            "📋",
            "#2F6FED",
            "把模糊想法收成能开工的方案：用户问题、范围、优先级和验收，不堆功能清单。",
            "你是产品经理，帮我把模糊想法收成工程师能直接动手的方案。\
先弄清三件事：用户是谁、要解决什么问题、做成什么样算成功；没交代就先问，别自己编一版人设。\
谈方案时把非目标、约束和取舍说在前面，优先级要讲为什么做这个、为什么现在不做那个。\
写需求就写场景、流程、边界和验收标准，不写空话口号。没证据的市场判断标成假设。\
需要结构时用表格或 diagram 技能；长文档协作用 doc-coauthoring，读附件用 docx。",
            &["doc-coauthoring", "docx", "diagram"],
        ),
        make(
            "asst_builtin_legal",
            "法务助手",
            "⚖️",
            "#8A6FBD",
            "合同和合规材料初审：标风险、提问题、给修改方向。审阅备忘，不替代律师。",
            "你是法务审阅搭档，帮我看合同、条款、隐私政策、用工和商务文件。\
先按红/黄/绿标风险，再说清楚：对我方意味着什么、常见改法、还要问对方什么。引用条款时带原文短摘，不凭记忆编法条编号。\
立场要问清（我方是买方、卖方、雇主还是平台），没说就先问一句。\
你不是律师，输出是审阅备忘不是法律意见；涉及诉讼、监管调查或大额交易，明确建议找持证律师复核。\
读附件用 pdf/docx 技能。",
            &["pdf", "docx"],
        ),
        make(
            "asst_builtin_finance",
            "财务分析",
            "💹",
            "#B7791F",
            "读报表和经营表格，做结构、比率与异常分析，结论落到数字。不做投资建议。",
            "你是财务分析搭档，读利润表、资产负债表、现金流和经营表格。\
先确认口径（期间、币种、是否合并、是否经审计），再拆结构、算关键比率、找异常和缺口。\
结论落到具体数字和对比，不写「整体向好」这种空话；你做的假设和数据质量问题主动摆出来。\
你不是持牌顾问，不给买卖、借贷或税务筹划建议。读表用 xlsx/pdf 技能，关系用 diagram 技能。",
            &["xlsx", "pdf", "diagram"],
        ),
        make(
            "asst_builtin_teacher",
            "教学助手",
            "📚",
            "#5E8C6A",
            "把难点拆开讲、把练习设计好：讲解、出题、找误区、做讲义和试卷。",
            "你是教学搭档，帮我讲清楚一个概念、出练习、改讲义或试卷。\
先确认学生大概程度和这节要达成什么，再讲；没交代就先问，别按竞赛难度一上来灌。\
讲解用例子和反例，练习带答案和常见错法，一次只攻一个难点。\
材料可做成讲义或试卷。读/写文档用 docx/pdf 技能，流程或知识结构用 diagram 技能。",
            &["docx", "pdf", "diagram"],
        ),
        make(
            "asst_builtin_reviewer",
            "代码审查",
            "🔎",
            "#3D5A80",
            "像资深工程师审变更：找能过测试却在生产炸掉的问题，按严重程度说。",
            "你是代码审查搭档，专找能过测试却在生产炸掉的问题：并发、错误处理、边界、权限、数据丢失、不可逆操作。\
先读变更和相关上下文，再按严重程度列问题；能给补丁就给小而聚焦的补丁，不借机重构。\
风格之争让给项目已有约定。没看过的代码不装看过，不确定的行为去代码里核实。\
架构或数据流需要讲清时用 diagram 技能。",
            &["diagram"],
        ),
        make(
            "asst_builtin_career",
            "求职教练",
            "🧳",
            "#7D6B5A",
            "简历、求职信和面试：按目标岗位改，写成可验证的事实，不编经历。",
            "你是求职教练，改简历、求职信、面试回答。\
先问目标岗位和真实经历要点，再按那个岗位改，不用万能模板。经历写成可验证的事实和结果，删掉空形容词。\
面试题给回答骨架和可能的追问，不替我编没做过的事。拿不准的空窗期或跳槽理由先问我。\
读原简历用 pdf/docx 技能。",
            &["pdf", "docx"],
        ),
    ]
}

/// 一次性内置专家迁移（v1）：用 `builtin_assistant_definitions` **覆盖整个**助手索引
/// （清空含用户自建的全部专家——这是用户明确选择），只留这 4 个内置专家。
///
/// 幂等性由调用方通过 `settings.builtin_assistants_seeded_v1` 标记保证；调用方必须在本函数
/// 成功后立即持久化该标记，否则下次启动会再次覆盖（连用户届时新建的专家一起抹掉）。
pub fn seed_builtin_assistants_v1(app: &AppHandle, now: i64) -> Result<(), String> {
    let index = ChatAssistantIndex {
        assistants: builtin_assistant_definitions(now),
    };
    save_assistant_index(app, &index)
}

/// 纯合并逻辑（便于单测）：按 id 把内置定义 upsert 进现有列表——
/// 同 id 项原位替换为新版，缺失的新内置按定义顺序追加，**其余条目（含用户自建）原样保留**。
pub(crate) fn merge_builtin_definitions(
    mut existing: Vec<ChatAssistant>,
    defs: Vec<ChatAssistant>,
) -> Vec<ChatAssistant> {
    let mut pending: std::collections::HashMap<String, ChatAssistant> =
        defs.iter().map(|d| (d.id.clone(), d.clone())).collect();
    // 原位替换已存在的同 id 内置项，保留其位置。
    for slot in existing.iter_mut() {
        if let Some(updated) = pending.remove(&slot.id) {
            *slot = updated;
        }
    }
    // 追加尚不存在的新内置项，保持定义顺序。
    for def in defs {
        if pending.contains_key(&def.id) {
            existing.push(def);
        }
    }
    existing
}

/// 非破坏性内置专家迁移（v2）：按 id upsert `builtin_assistant_definitions`，更新旧内置、
/// 补齐新增内置，**保留用户自建/非内置条目**。与 v1 的整表覆盖不同，可安全对已 seed v1 的
/// 老用户重跑一次。幂等由调用方通过 `settings.builtin_assistants_seeded_v2` 标记保证。
pub fn merge_builtin_assistants_v2(app: &AppHandle, now: i64) -> Result<(), String> {
    let existing = load_assistant_index(app)?.assistants;
    let merged = merge_builtin_definitions(existing, builtin_assistant_definitions(now));
    save_assistant_index(app, &ChatAssistantIndex { assistants: merged })
}

/// 非破坏性内置专家迁移（v3）：与 v2 同一套 upsert，补齐产品/法务/财务/教学/审查/求职。
/// 幂等由调用方通过 `settings.builtin_assistants_seeded_v3` 标记保证。
pub fn merge_builtin_assistants_v3(app: &AppHandle, now: i64) -> Result<(), String> {
    merge_builtin_assistants_v2(app, now)
}

pub fn get_assistants(
    app: &AppHandle,
    include_archived: bool,
) -> Result<Vec<ChatAssistant>, String> {
    let index = load_assistant_index(app)?;
    let mut assistants = index.assistants;
    if !include_archived {
        assistants.retain(|assistant| !assistant.archived);
    }
    assistants.sort_by(|a, b| {
        b.updated_at
            .cmp(&a.updated_at)
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(assistants)
}

pub fn get_assistant(app: &AppHandle, assistant_id: &str) -> Result<ChatAssistant, String> {
    validate_assistant_id(assistant_id)?;
    load_assistant_index(app)?
        .assistants
        .into_iter()
        .find(|assistant| assistant.id == assistant_id)
        .ok_or_else(|| "助手不存在".to_string())
}

pub fn create_assistant(
    app: &AppHandle,
    mut assistant: ChatAssistant,
) -> Result<ChatAssistant, String> {
    validate_assistant_id(&assistant.id)?;
    normalize_assistant(&mut assistant)?;
    let mut index = load_assistant_index(app)?;
    if index.assistants.iter().any(|item| item.id == assistant.id) {
        return Err("助手 ID 已存在".to_string());
    }
    if index
        .assistants
        .iter()
        .any(|item| !item.archived && item.name == assistant.name)
    {
        return Err("助手名称已存在".to_string());
    }
    index.assistants.insert(0, assistant.clone());
    save_assistant_index(app, &index)?;
    Ok(assistant)
}

pub fn update_assistant(
    app: &AppHandle,
    assistant: ChatAssistant,
) -> Result<ChatAssistant, String> {
    validate_assistant_id(&assistant.id)?;
    let mut next = assistant;
    normalize_assistant(&mut next)?;
    let mut index = load_assistant_index(app)?;
    let pos = index
        .assistants
        .iter()
        .position(|item| item.id == next.id)
        .ok_or_else(|| "助手不存在".to_string())?;
    if index
        .assistants
        .iter()
        .any(|item| item.id != next.id && !item.archived && item.name == next.name)
    {
        return Err("助手名称已存在".to_string());
    }
    next.built_in = index.assistants[pos].built_in;
    next.created_at = index.assistants[pos].created_at;
    index.assistants[pos] = next.clone();
    save_assistant_index(app, &index)?;
    Ok(next)
}

pub fn duplicate_assistant(app: &AppHandle, assistant_id: &str) -> Result<ChatAssistant, String> {
    let source = get_assistant(app, assistant_id)?;
    let now = chrono::Local::now().timestamp();
    let copy = ChatAssistant {
        id: format!("asst_{}", uuid::Uuid::new_v4()),
        name: unique_assistant_copy_name(app, &source.name)?,
        built_in: false,
        archived: false,
        created_at: now,
        updated_at: now,
        ..source
    };
    create_assistant(app, copy)
}

pub fn archive_assistant(app: &AppHandle, assistant_id: &str) -> Result<(), String> {
    validate_assistant_id(assistant_id)?;
    let mut index = load_assistant_index(app)?;
    let Some(pos) = index
        .assistants
        .iter()
        .position(|assistant| assistant.id == assistant_id)
    else {
        return Err("助手不存在".to_string());
    };
    index.assistants[pos].archived = true;
    index.assistants[pos].updated_at = chrono::Local::now().timestamp();
    save_assistant_index(app, &index)
}

pub fn assistant_snapshot(
    app: &AppHandle,
    assistant_id: &str,
) -> Result<ChatAssistantSnapshot, String> {
    let assistant = get_assistant(app, assistant_id)?;
    if !assistant_is_available(&assistant) {
        return Err("助手不可用".to_string());
    }
    Ok(ChatAssistantSnapshot::from(&assistant))
}

/// `enabled` 是旧版助手中心留下的兼容字段。新版以「常用 / installed」控制选择器展示，
/// 不再提供启停入口；若继续把旧的 `enabled=false` 当作运行时禁用，用户会看到助手却无法使用，
/// 也没有任何地方能重新启用。因此运行时可用性只由归档状态决定。
pub(super) fn assistant_is_available(assistant: &ChatAssistant) -> bool {
    !assistant.archived
}

fn normalize_assistant(assistant: &mut ChatAssistant) -> Result<(), String> {
    assistant.name = assistant.name.trim().to_string();
    if assistant.name.is_empty() {
        return Err("助手名称不能为空".to_string());
    }
    if assistant.name.chars().count() > 64 {
        return Err("助手名称不能超过 64 个字符".to_string());
    }
    assistant.description = assistant.description.trim().to_string();
    if assistant.description.chars().count() > 240 {
        return Err("助手描述不能超过 240 个字符".to_string());
    }
    assistant.icon = assistant.icon.trim().chars().take(8).collect();
    assistant.color = assistant.color.trim().chars().take(32).collect();
    assistant.source = normalize_assistant_source(&assistant.source, assistant.built_in);
    assistant.system_prompt = assistant.system_prompt.trim().to_string();
    assistant.provider_id = assistant.provider_id.trim().to_string();
    assistant.model = assistant.model.trim().to_string();
    assistant.mcp_server_ids = normalize_string_list(&assistant.mcp_server_ids, 64, 200);
    canonicalize_cua_mcp_server_ids(&mut assistant.mcp_server_ids);
    assistant.skill_ids = normalize_string_list(&assistant.skill_ids, 64, 200);
    Ok(())
}

pub(super) fn canonicalize_cua_mcp_server_ids(ids: &mut Vec<String>) {
    for id in ids.iter_mut() {
        if id.trim() == crate::computer_control::LEGACY_CUA_MCP_SERVER_ID {
            *id = crate::computer_control::CUA_MCP_SERVER_ID.to_string();
        }
    }
    let mut seen = std::collections::HashSet::new();
    ids.retain(|id| seen.insert(id.clone()));
}

fn normalize_assistant_source(source: &str, built_in: bool) -> String {
    match source.trim() {
        "builtin" | "user" | "imported" => source.trim().to_string(),
        _ if built_in => "builtin".to_string(),
        _ => "user".to_string(),
    }
}

fn normalize_string_list(values: &[String], limit: usize, max_chars: usize) -> Vec<String> {
    let mut out = Vec::new();
    for value in values {
        let item: String = value.trim().chars().take(max_chars).collect();
        if item.is_empty() || out.iter().any(|existing| existing == &item) {
            continue;
        }
        out.push(item);
        if out.len() >= limit {
            break;
        }
    }
    out
}

fn unique_assistant_copy_name(app: &AppHandle, base_name: &str) -> Result<String, String> {
    let index = load_assistant_index(app)?;
    let base = format!("{base_name} 副本");
    if !index
        .assistants
        .iter()
        .any(|assistant| !assistant.archived && assistant.name == base)
    {
        return Ok(base);
    }
    for i in 2..100 {
        let candidate = format!("{base} {i}");
        if !index
            .assistants
            .iter()
            .any(|assistant| !assistant.archived && assistant.name == candidate)
        {
            return Ok(candidate);
        }
    }
    Ok(format!("{base} {}", chrono::Local::now().timestamp()))
}
