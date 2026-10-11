import { forwardRef, type ButtonHTMLAttributes, type ReactNode } from 'react'

/**
 * 统一动作按钮。内部输出既有的 `kv-btn` CSS 类（见 src/index.css），
 * 作为全项目按钮的单一 React 来源。
 *
 * 用法：新的**动作按钮**（CTA / 工具栏 / 对话框按钮）一律用 <Button> / <IconButton>，
 * 不要再手写内联按钮样式。列表行 / 菜单项 / 分段控件·tab / 窗口控件 / 发送键
 * 是各自独立的交互模式，不适用本组件。
 */

type ButtonVariant = 'default' | 'primary' | 'ghost' | 'danger'
type ButtonSize = 'md' | 'sm'

type ButtonProps = {
  variant?: ButtonVariant
  size?: ButtonSize
  children?: ReactNode
} & ButtonHTMLAttributes<HTMLButtonElement>

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  {
    variant = 'default',
    size = 'md',
    type = 'button',
    className = '',
    children,
    ...props
  },
  ref,
) {
  const classes = [
    'kv-btn',
    variant !== 'default' && variant,
    size === 'sm' && 'sm',
    className,
  ]
    .filter(Boolean)
    .join(' ')
  return (
    <button ref={ref} type={type} className={classes} {...props}>
      {children}
    </button>
  )
})

type IconButtonVariant = 'default' | 'ghost' | 'danger'
type IconButtonSize = 'xs' | 'sm' | 'md' | 'lg'
type IconButtonShape = 'square' | 'circle'

type IconButtonProps = {
  variant?: IconButtonVariant
  /** xs=22（裸基类，设置域既有用法）/ sm=28 / md=32 / lg=36（外框尺寸由 `.kv-icon-btn.<size>` 控制）。 */
  size?: IconButtonSize
  shape?: IconButtonShape
  /** 必填：同时用作 aria-label 与 title，保证可访问性。 */
  label: string
  children: ReactNode
} & Omit<ButtonHTMLAttributes<HTMLButtonElement>, 'aria-label'>

export const IconButton = forwardRef<HTMLButtonElement, IconButtonProps>(function IconButton(
  {
    variant = 'default',
    size = 'sm',
    shape = 'square',
    label,
    type = 'button',
    className = '',
    title,
    children,
    ...props
  },
  ref,
) {
  const classes = [
    'kv-icon-btn',
    size !== 'xs' && size,
    shape === 'circle' && 'circle',
    variant !== 'default' && variant,
    className,
  ]
    .filter(Boolean)
    .join(' ')
  return (
    <button ref={ref} type={type} className={classes} aria-label={label} title={title ?? label} {...props}>
      {children}
    </button>
  )
})
