function preventNativeContextMenu(event: MouseEvent) {
  event.preventDefault()
}

function preventNativeFind(event: KeyboardEvent) {
  if (event.altKey || event.isComposing) return
  const key = event.key.toLowerCase()
  if (key === 'f3' || ((event.ctrlKey || event.metaKey) && (key === 'f' || key === 'g'))) {
    // 只取消浏览器查找，继续传递给阅读器等应用内搜索。
    event.preventDefault()
  }
}

document.addEventListener('contextmenu', preventNativeContextMenu)
window.addEventListener('keydown', preventNativeFind, true)

if (import.meta.hot) import.meta.hot.dispose(() => {
  document.removeEventListener('contextmenu', preventNativeContextMenu)
  window.removeEventListener('keydown', preventNativeFind, true)
})
