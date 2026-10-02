// 设置导航的自定义线性图标。接口对齐 lucide-react（size / strokeWidth / className），
// 可直接替换 <Icon size={17} strokeWidth={1.75} />。fill=none + stroke=currentColor，
// 颜色随上层文字色（选中态 / 深色模式自动跟随）。

import './NavIcons.css'

interface IconProps {
  size?: number
  strokeWidth?: number
  className?: string
}

function svgProps({ size = 24, strokeWidth = 2, className }: IconProps) {
  return {
    width: size,
    height: size,
    viewBox: '0 0 24 24',
    fill: 'none',
    stroke: 'currentColor',
    strokeWidth,
    strokeLinecap: 'round' as const,
    strokeLinejoin: 'round' as const,
    className,
  }
}

// 基础：八齿齿轮 + 轴孔
export function GeneralIcon(props: IconProps) {
  return (
    <svg {...svgProps(props)}>
      <path d="M10.4 5.08 L10.24 2.97 A9.2 9.2 0 0 1 13.76 2.97 L13.6 5.08 A7.1 7.1 0 0 1 15.76 5.98 L17.14 4.37 A9.2 9.2 0 0 1 19.63 6.86 L18.02 8.24 A7.1 7.1 0 0 1 18.92 10.4 L21.03 10.24 A9.2 9.2 0 0 1 21.03 13.76 L18.92 13.6 A7.1 7.1 0 0 1 18.02 15.76 L19.63 17.14 A9.2 9.2 0 0 1 17.14 19.63 L15.76 18.02 A7.1 7.1 0 0 1 13.6 18.92 L13.76 21.03 A9.2 9.2 0 0 1 10.24 21.03 L10.4 18.92 A7.1 7.1 0 0 1 8.24 18.02 L6.86 19.63 A9.2 9.2 0 0 1 4.37 17.14 L5.98 15.76 A7.1 7.1 0 0 1 5.08 13.6 L2.97 13.76 A9.2 9.2 0 0 1 2.97 10.24 L5.08 10.4 A7.1 7.1 0 0 1 5.98 8.24 L4.37 6.86 A9.2 9.2 0 0 1 6.86 4.37 L8.24 5.98 A7.1 7.1 0 0 1 10.4 5.08 Z" />
      <circle cx="12" cy="12" r="2.6" />
    </svg>
  )
}


// 快捷键：⌘ 键符号
export function HotkeysIcon(props: IconProps) {
  return (
    <svg {...svgProps(props)}>
      <path d="M9.25 9.25 V14.75 M14.75 9.25 V14.75 M9.25 9.25 H14.75 M9.25 14.75 H14.75" />
      <path d="M9.25 9.25 H7 A2.25 2.25 0 1 1 9.25 7 Z M14.75 9.25 V7 A2.25 2.25 0 1 1 17 9.25 Z M14.75 14.75 H17 A2.25 2.25 0 1 1 14.75 17 Z M9.25 14.75 V17 A2.25 2.25 0 1 1 7 14.75 Z" />
    </svg>
  )
}


// 知识库：摊开的书（两页微弧 + 书脊）
export function KnowledgeIcon(props: IconProps) {
  return (
    <svg {...svgProps(props)}>
      <path d="M12 6.75 C10 5.25 7 4.75 3.5 5.25 V18.75 C7 18.25 10 18.75 12 20.25 C14 18.75 17 18.25 20.5 18.75 V5.25 C17 4.75 14 5.25 12 6.75 Z" />
      <path d="M12 6.75 V20.25" />
    </svg>
  )
}

// 输入翻译：前卡 A，后卡露出一角的「文」
export function TranslateIcon(props: IconProps) {
  return (
    <svg {...svgProps(props)}>
      <path d="M14.5 8.5 H18.5 A2 2 0 0 1 20.5 10.5 V18.5 A2 2 0 0 1 18.5 20.5 H10.5 A2 2 0 0 1 8.5 18.5 V14.5" />
      <rect x="3.5" y="3.5" width="11" height="11" rx="2.5" />
      <path d="M6.75 11.75 L9 6.5 L11.25 11.75 M7.6 10 H10.4" />
      <path d="M13 14.5 H18.5 M17 14.5 Q16.5 17.5 13 18.5 M14.25 15.75 Q15.5 18 18.5 18.5" />
    </svg>
  )
}


// 快速翻译：闪电
export function ScreenshotIcon(props: IconProps) {
  return (
    <svg {...svgProps(props)}>
      <path d="M13 2 L4 14 H11 L11 22 L20 10 H13 Z" />
    </svg>
  )
}

// Lens：圆角取景框 + 镜头 + 对焦点
export function LensIcon(props: IconProps) {
  return (
    <svg {...svgProps(props)}>
      <path d="M3.5 8.5 V6.5 A3 3 0 0 1 6.5 3.5 H8.5" />
      <path d="M15.5 3.5 H17.5 A3 3 0 0 1 20.5 6.5 V8.5" />
      <path d="M20.5 15.5 V17.5 A3 3 0 0 1 17.5 20.5 H15.5" />
      <path d="M8.5 20.5 H6.5 A3 3 0 0 1 3.5 17.5 V15.5" />
      <circle cx="12" cy="12" r="3.75" />
      <circle cx="12" cy="12" r="0.9" fill="currentColor" stroke="none" />
    </svg>
  )
}


// AI 客户端：应用 logo mark（与 RuntimePicker / 内置 Agent 一致）
export function ChatIcon({ size = 24, className }: IconProps) {
  return (
    <img
      src="/logo-mark.png"
      alt=""
      aria-hidden="true"
      width={size}
      height={size}
      className={['kv-nav-logo-mark', className].filter(Boolean).join(' ')}
      draggable={false}
    />
  )
}

// 记忆：三层叠片（L1 注入 / L2 按需），顶层带标记点
export function MemoryIcon(props: IconProps) {
  return (
    <svg {...svgProps(props)}>
      <path d="M12 3.5 L20.5 8 L12 12.5 L3.5 8 Z" />
      <path d="M3.5 12 L12 16.5 L20.5 12" />
      <path d="M3.5 16 L12 20.5 L20.5 16" />
      <circle cx="12" cy="8" r="1" fill="currentColor" stroke="none" />
    </svg>
  )
}


// 混音器：三条推子轨道，胶囊形推钮断开轨道
export function MixerIcon(props: IconProps) {
  return (
    <svg {...svgProps(props)}>
      <path d="M6 3.5 V8 M6 12 V20.5" />
      <path d="M12 3.5 V12.5 M12 16.5 V20.5" />
      <path d="M18 3.5 V6 M18 10 V20.5" />
      <rect x="3.75" y="8" width="4.5" height="4" rx="1.5" />
      <rect x="9.75" y="12.5" width="4.5" height="4" rx="1.5" />
      <rect x="15.75" y="6" width="4.5" height="4" rx="1.5" />
    </svg>
  )
}


// 本地 CLI：终端窗口（标题栏 + 提示符 + 光标）
export function CliIcon(props: IconProps) {
  return (
    <svg {...svgProps(props)}>
      <rect x="3" y="4" width="18" height="16" rx="3" />
      <path d="M3 8 H21" />
      <path d="M7 12 L9.5 14 L7 16" />
      <path d="M12 16 H16" />
    </svg>
  )
}


// 助手：圆角机器人头（天线 + 胶囊眼 + 耳侧）
export function AgentIcon(props: IconProps) {
  return (
    <svg {...svgProps(props)}>
      <rect x="4.5" y="7.5" width="15" height="12" rx="4" />
      <path d="M12 7.5 V4.75" />
      <circle cx="12" cy="3.75" r="1" fill="currentColor" stroke="none" />
      <path d="M9.25 12.25 V14 M14.75 12.25 V14" />
      <path d="M2.5 12 V15 M21.5 12 V15" />
    </svg>
  )
}

// MCP：插头/连接
export function McpIcon(props: IconProps) {
  return (
    <svg {...svgProps(props)}>
      <path d="M9 3 V7" />
      <path d="M15 3 V7" />
      <path d="M7 7 H17 V11 A5 5 0 0 1 7 11 Z" />
      <path d="M12 16 V21" />
    </svg>
  )
}

// 连接器：两个斜向互扣的链环
export function ConnectorsIcon(props: IconProps) {
  return (
    <svg {...svgProps(props)}>
      <g transform="rotate(-45 12 12)">
        <rect x="1.25" y="8.5" width="12.5" height="7" rx="3.5" />
        <rect x="10.25" y="8.5" width="12.5" height="7" rx="3.5" />
      </g>
    </svg>
  )
}


// 作品：扁封套，口里露出折角 / 方片 / 圆片
export function WorksIcon(props: IconProps) {
  return (
    <svg {...svgProps(props)}>
      <path d="M4 9.2 H20 A1.4 1.4 0 0 1 21.4 10.6 V18.6 A1.6 1.6 0 0 1 19.8 20.2 H4.2 A1.6 1.6 0 0 1 2.6 18.6 V10.6 A1.4 1.4 0 0 1 4 9.2 Z" />
      <path d="M7.2 9.2 V6.4 A1.5 1.5 0 0 1 8.7 4.9 H11.1" />
      <rect x="12.2" y="3.6" width="4.6" height="5.6" rx="1.3" />
      <circle cx="18.8" cy="6.2" r="1.35" />
    </svg>
  )
}

// 对话库：前景气泡带尾巴，后面一枚气泡露出右上角
export function SessionsIcon(props: IconProps) {
  return (
    <svg {...svgProps(props)}>
      <path d="M8.5 6 V5.5 A2 2 0 0 1 10.5 3.5 H18.5 A2 2 0 0 1 20.5 5.5 V11.5 A2 2 0 0 1 18.5 13.5 H18.5" />
      <path d="M5.5 8 H14.5 A2 2 0 0 1 16.5 10 V15.5 A2 2 0 0 1 14.5 17.5 H9.5 L6 20.5 V17.5 H5.5 A2 2 0 0 1 3.5 15.5 V10 A2 2 0 0 1 5.5 8 Z" />
      <path d="M7 12.75 H13" />
    </svg>
  )
}


// 插件：拼图块（与扩展中心里原 Puzzle 语义一致）
export function PluginsIcon(props: IconProps) {
  return (
    <svg {...svgProps(props)}>
      <path d="M12 3.5 V6.5" />
      <path d="M12 17.5 V20.5" />
      <path d="M3.5 12 H6.5" />
      <path d="M17.5 12 H20.5" />
      <path d="M8.5 6.5 H11 A1.5 1.5 0 1 1 14 6.5 H15.5 A2 2 0 0 1 17.5 8.5 V11 A1.5 1.5 0 1 1 17.5 14 V15.5 A2 2 0 0 1 15.5 17.5 H14 A1.5 1.5 0 1 1 11 17.5 H8.5 A2 2 0 0 1 6.5 15.5 V14 A1.5 1.5 0 1 1 6.5 11 V8.5 A2 2 0 0 1 8.5 6.5 Z" />
    </svg>
  )
}

// Hooks：一枚鱼钩（环眼 + 钩身 + 倒刺）
export function HooksIcon(props: IconProps) {
  return (
    <svg {...svgProps(props)}>
      <circle cx="16" cy="4.75" r="1.75" />
      <path d="M16 6.5 V14 A5.75 5.75 0 0 1 4.5 14 V11 L8 13.75" />
    </svg>
  )
}


// 插件：主体模块右上留出接口，独立圆角模块嵌入。
export function PluginIcon(props: IconProps) {
  return (
    <svg {...svgProps(props)} aria-hidden="true">
      <path d="M10 4.5 H6 A2.5 2.5 0 0 0 3.5 7 V18 A2.5 2.5 0 0 0 6 20.5 H17 A2.5 2.5 0 0 0 19.5 18 V14 H12.5 A2.5 2.5 0 0 1 10 11.5 Z" />
      <rect data-nav-motion="plugin" x="14" y="3.5" width="7" height="7" rx="1.75" />
    </svg>
  )
}

export function ComposeIcon(props: IconProps) {
  return (
    <svg {...svgProps(props)} aria-hidden="true">
      <path d="M10 4.5 H6 A2 2 0 0 0 4 6.5 V18 A2 2 0 0 0 6 20 H17.5 A2 2 0 0 0 19.5 18 V14" />
      <g data-nav-motion="compose">
        <path d="m10 11 7.5-7.5 a1.8 1.8 0 0 1 2.5 2.5 L12.5 13.5 9 14 Z" />
        <path d="m16 5 2.5 2.5" />
      </g>
    </svg>
  )
}

export function SearchNavIcon(props: IconProps) {
  return (
    <svg {...svgProps(props)} aria-hidden="true">
      <g data-nav-motion="search">
        <circle cx="10.5" cy="10.5" r="6.5" />
        <path d="m15.2 15.2 4.8 4.8" />
      </g>
    </svg>
  )
}

export function AutomationIcon(props: IconProps) {
  return (
    <svg {...svgProps(props)} aria-hidden="true">
      <circle cx="12" cy="12" r="8.5" />
      <path d="M12 12 H16" />
      <path data-nav-motion="automation" d="M12 12 V6.5" />
    </svg>
  )
}

export function ScheduleIcon(props: IconProps) {
  return (
    <svg {...svgProps(props)} aria-hidden="true">
      <circle cx="12" cy="12" r="8.5" />
      <path d="M12 6.5 V12 L16 14.5" />
    </svg>
  )
}

// 任务：清单与完成标记，涵盖定时任务和自动化工作流。
export function TasksIcon(props: IconProps) {
  return (
    <svg {...svgProps(props)} aria-hidden="true">
      <rect x="5" y="4" width="14" height="17" rx="2.5" />
      <path d="M9 3.5 H15 V6.5 H9 Z" />
      <path d="M8.5 12 L10 13.5 L12.5 10.5 M14.5 12 H16 M8.5 17 H16" />
    </svg>
  )
}

export function PortfolioIcon(props: IconProps) {
  return (
    <svg {...svgProps(props)} aria-hidden="true">
      <path data-nav-motion="portfolio-back" d="M7 3.5 H17" />
      <path data-nav-motion="portfolio-middle" d="M5 7 H19" />
      <rect data-nav-motion="portfolio-front" x="3.5" y="10.5" width="17" height="10" rx="2" />
    </svg>
  )
}

export function ExtensionsIcon(props: IconProps) {
  return (
    <svg {...svgProps(props)} aria-hidden="true">
      <g data-nav-motion="extensions-a">
        <rect x="3.5" y="3.5" width="6.5" height="6.5" rx="1.5" />
        <rect x="14" y="14" width="6.5" height="6.5" rx="1.5" />
      </g>
      <g data-nav-motion="extensions-b">
        <rect x="14" y="3.5" width="6.5" height="6.5" rx="1.5" />
        <rect x="3.5" y="14" width="6.5" height="6.5" rx="1.5" />
      </g>
    </svg>
  )
}

// Skill：卷轴
export function SkillIcon(props: IconProps) {
  return (
    <svg {...svgProps(props)}>
      <path d="M15 8 H10" />
      <path d="M15 12 H10" />
      <path d="M19 17 V5 A2 2 0 0 0 17 3 H4" />
      <path d="M8 21 H20 A2 2 0 0 0 22 19 V18 A1 1 0 0 0 21 17 H11 A1 1 0 0 0 10 18 V19 A2 2 0 1 1 6 19 V5 A2 2 0 1 0 2 5 V7 A1 1 0 0 0 3 8 H6" />
    </svg>
  )
}

// 插件 / Skill 没有自带图标时的占位图标（插件市场、Skill 卡片）。不复用导航图标，避免列表看起来像菜单入口。
// 插件：一块乐高积木（顶面两颗凸点）
export function DefaultPluginIcon(props: IconProps) {
  return (
    <svg {...svgProps(props)} aria-hidden="true">
      <path d="M6 8.5 V6.5 A1 1 0 0 1 7 5.5 H9 A1 1 0 0 1 10 6.5 V8.5" />
      <path d="M14 8.5 V6.5 A1 1 0 0 1 15 5.5 H17 A1 1 0 0 1 18 6.5 V8.5" />
      <rect x="3.5" y="8.5" width="17" height="10" rx="1.75" />
    </svg>
  )
}

// Skill：一大一小两颗内凹四角星
export function DefaultSkillIcon(props: IconProps) {
  return (
    <svg {...svgProps(props)} aria-hidden="true">
      <path d="M10 3 C10.6 7 13 9.4 17 10 C13 10.6 10.6 13 10 17 C9.4 13 7 10.6 3 10 C7 9.4 9.4 7 10 3 Z" />
      <path d="M18 14 C18.2 15.6 19.4 16.8 21 17 C19.4 17.2 18.2 18.4 18 20 C17.8 18.4 16.6 17.2 15 17 C16.6 16.8 17.8 15.6 18 14 Z" />
    </svg>
  )
}

// 网络搜索：地球仪（经线椭圆 + 弯曲纬线），填满 viewBox，小尺寸也清晰
export function WebSearchIcon(props: IconProps) {
  return (
    <svg {...svgProps(props)}>
      <circle cx="12" cy="12" r="8.75" />
      <path d="M12 3.25 a 4 8.75 0 0 0 0 17.5 a 4 8.75 0 0 0 0 -17.5" />
      <path d="M4.2 8.5 Q12 10.6 19.8 8.5" />
      <path d="M4.2 15.5 Q12 13.4 19.8 15.5" />
    </svg>
  )
}


// 用量统计：三根圆角柱
export function UsageIcon(props: IconProps) {
  return (
    <svg {...svgProps(props)}>
      <rect x="4" y="12" width="4" height="8.5" rx="1.25" />
      <rect x="10" y="4" width="4" height="16.5" rx="1.25" />
      <rect x="16" y="8.5" width="4" height="12" rx="1.25" />
    </svg>
  )
}


// 模型：等轴立方体（一个模型 = 一个封装好的块），与齿轮式的「基础」拉开轮廓
export function ProvidersIcon(props: IconProps) {
  return (
    <svg {...svgProps(props)}>
      <path d="M12 2.75 L20 7.25 V16.75 L12 21.25 L4 16.75 V7.25 Z" />
      <path d="M4 7.25 L12 11.75 L20 7.25" />
      <path d="M12 11.75 V21.25" />
    </svg>
  )
}


// 关于：信息（实心点）
export function AboutIcon(props: IconProps) {
  return (
    <svg {...svgProps(props)}>
      <circle cx="12" cy="12" r="8.75" />
      <path d="M12 11 V16.25" />
      <circle cx="12" cy="7.85" r="1.1" fill="currentColor" stroke="none" />
    </svg>
  )
}

// 电脑操控：显示器里一枚指针
export function ComputerIcon(props: IconProps) {
  return (
    <svg {...svgProps(props)}>
      <rect x="3" y="3.5" width="18" height="13" rx="2.5" />
      <path d="M9 20.5 H15 M12 16.5 V20.5" />
      <path d="M10 7 L14.8 9.4 L12.7 10.2 L11.8 12.4 Z" />
    </svg>
  )
}

// 媒体站：后方一张相片，前方一块带播放键的画面
export function MediaIcon(props: IconProps) {
  return (
    <svg {...svgProps(props)}>
      <path d="M7 6.5 V5.5 A2 2 0 0 1 9 3.5 H18.5 A2 2 0 0 1 20.5 5.5 V13 A2 2 0 0 1 18.5 15 H17.5" />
      <rect x="3.5" y="8" width="14" height="12.5" rx="2.5" />
      <path d="M9 11.75 V16.75 L13.25 14.25 Z" />
    </svg>
  )
}

// 笔记：折角便签 + 两行字
export function NotesIcon(props: IconProps) {
  return (
    <svg {...svgProps(props)}>
      <path d="M14.5 20.5 H6.5 A2 2 0 0 1 4.5 18.5 V5.5 A2 2 0 0 1 6.5 3.5 H17.5 A2 2 0 0 1 19.5 5.5 V15.5 Z" />
      <path d="M14.5 20.5 V17.5 A2 2 0 0 1 16.5 15.5 H19.5" />
      <path d="M8 8.5 H16 M8 12 H13" />
    </svg>
  )
}
