import { Component, type ReactNode } from 'react'

type MarkdownErrorBoundaryProps = {
  children: ReactNode
  fallbackText: string
}

type MarkdownErrorBoundaryState = {
  failed: boolean
}

export class MarkdownErrorBoundary extends Component<
  MarkdownErrorBoundaryProps,
  MarkdownErrorBoundaryState
> {
  state: MarkdownErrorBoundaryState = { failed: false }

  static getDerivedStateFromError(): MarkdownErrorBoundaryState {
    return { failed: true }
  }

  render() {
    if (this.state.failed) {
      return (
        <pre className="whitespace-pre-wrap break-words text-[15px] leading-relaxed text-neutral-900 dark:text-neutral-100">
          {this.props.fallbackText}
        </pre>
      )
    }

    return this.props.children
  }
}
