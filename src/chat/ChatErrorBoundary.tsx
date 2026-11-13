import { Component, type ErrorInfo, type ReactNode } from 'react'
import { Button } from '../components/Button'

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
        <div className="flex h-full min-h-0 w-full flex-col items-center justify-center bg-neutral-50 px-6">
          <div className="max-w-md text-center">
            <h2 className="text-base font-semibold text-neutral-900">
              界面加载失败
            </h2>
            <p className="mt-2 break-all text-sm text-danger">
              {this.state.error.message}
            </p>
            <Button className="mt-4" onClick={() => this.setState({ error: null })}>
              重试
            </Button>
          </div>
        </div>
      )
    }

    return this.props.children
  }
}
