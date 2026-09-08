/** Report state from DOM events so reply completion never has to query WebKit. */
export function observeNotificationView(report: (route: string, viewing: boolean) => Promise<void>) {
  const send = (viewing: boolean) => {
    void report(window.location.hash, viewing).catch(() => {
      // Notification suppression must never interrupt normal window interaction.
    })
  }
  const sync = () => send(document.visibilityState === 'visible' && document.hasFocus())
  const clear = () => send(false)
  window.addEventListener('focus', sync)
  window.addEventListener('blur', clear)
  window.addEventListener('hashchange', sync)
  window.addEventListener('pageshow', sync)
  window.addEventListener('pagehide', clear)
  document.addEventListener('visibilitychange', sync)
  sync()
  return () => {
    window.removeEventListener('focus', sync)
    window.removeEventListener('blur', clear)
    window.removeEventListener('hashchange', sync)
    window.removeEventListener('pageshow', sync)
    window.removeEventListener('pagehide', clear)
    document.removeEventListener('visibilitychange', sync)
    clear()
  }
}
