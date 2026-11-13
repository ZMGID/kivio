import { Component, type ErrorInfo, type ReactNode } from 'react'

type ToolCallErrorBoundaryProps = {
  children: ReactNode
}

type ToolCallErrorBoundaryState = {
  failed: boolean
}

export class ToolCallErrorBoundary extends Component<
  ToolCallErrorBoundaryProps,
  ToolCallErrorBoundaryState
> {
  state: ToolCallErrorBoundaryState = { failed: false }

  static getDerivedStateFromError(): ToolCallErrorBoundaryState {
    return { failed: true }
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error('[Chat] tool call render error:', error, info.componentStack)
  }

  render() {
    if (this.state.failed) {
      return (
        <div className="not-prose mb-2 text-[12px] leading-5 text-red-500 dark:text-red-400">
          工具调用显示失败
        </div>
      )
    }

    return this.props.children
  }
}
