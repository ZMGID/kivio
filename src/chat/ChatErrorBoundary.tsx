import { Component, type ErrorInfo, type ReactNode } from 'react'

type ChatErrorBoundaryProps = {
  children: ReactNode
}

type ChatErrorBoundaryState = {
  error: Error | null
}

export class ChatErrorBoundary extends Component<ChatErrorBoundaryProps, ChatErrorBoundaryState> {
  state: ChatErrorBoundaryState = { error: null }

  static getDerivedStateFromError(error: Error): ChatErrorBoundaryState {
    return { error }
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error('[Chat] render error:', error, info.componentStack)
  }

  render() {
    if (this.state.error) {
      return (
        <div className="flex h-full min-h-0 w-full flex-col items-center justify-center bg-white px-6 dark:bg-[#212121]">
          <div className="max-w-md text-center">
            <h2 className="text-base font-semibold text-neutral-900 dark:text-neutral-100">
              界面加载失败
            </h2>
            <p className="mt-2 break-all text-sm text-red-600 dark:text-red-400">
              {this.state.error.message}
            </p>
            <button
              type="button"
              className="mt-4 rounded-full border border-neutral-200/90 bg-white px-4 py-2 text-sm font-medium text-neutral-800 shadow-sm hover:bg-neutral-50 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-100 dark:hover:bg-neutral-800"
              onClick={() => this.setState({ error: null })}
            >
              重试
            </button>
          </div>
        </div>
      )
    }

    return this.props.children
  }
}
