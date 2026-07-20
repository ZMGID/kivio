// 消息区「添加到聊天」→ 输入框 的单监听信道。
// 同一时刻只有一个活跃 composer（InputBar），故单监听足够。
// ponytail: single listener; 若将来同屏多 composer 再改成 Set。
type Listener = (text: string) => void

let listener: Listener | null = null

export function onComposerInsert(cb: Listener): () => void {
  listener = cb
  return () => {
    if (listener === cb) listener = null
  }
}

export function insertIntoComposer(text: string): void {
  listener?.(text)
}
