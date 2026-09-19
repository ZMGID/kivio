/** Window chrome capabilities shared by feature shells. */
export const isMac =
  typeof navigator !== 'undefined' && /Mac|iPhone|iPad|iPod/i.test(navigator.userAgent)

export const isWindows =
  typeof navigator !== 'undefined' && /Windows/i.test(navigator.userAgent)

export const usesNativeTitlebar = isMac
