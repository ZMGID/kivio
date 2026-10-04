// Loaded synchronously before the application bundle. This is a paint cache,
// not settings: the authoritative backend snapshot replaces it after startup.
(function () {
  var root = document.documentElement
  var palette = null
  try {
    var cached = JSON.parse(window.localStorage.getItem('kivio.theme.startup.v1'))
    if (cached && cached.version === 1 && ['system', 'light', 'dark'].indexOf(cached.mode) !== -1 && typeof cached.themeColor === 'string') {
      var dark = cached.mode === 'dark' || (cached.mode === 'system' && window.matchMedia('(prefers-color-scheme: dark)').matches)
      var candidate = dark ? cached.dark : cached.light
      if (candidate && typeof candidate === 'object' && !Array.isArray(candidate) &&
        ['--theme-surface', '--text', '--accent'].every(function (key) { return Object.prototype.hasOwnProperty.call(candidate, key) }) &&
        Object.keys(candidate).every(function (key) {
          return (key === 'color' || /^--[a-z][a-z0-9-]*$/.test(key)) && /^#[0-9a-f]{6}$/i.test(candidate[key])
        })) {
        palette = candidate
        Object.keys(palette).forEach(function (key) { root.style.setProperty(key, palette[key]) })
        root.classList.toggle('dark', dark)
        root.dataset.themeColor = cached.themeColor
      }
    }
  } catch (_) {
    // Storage may be unavailable or contain an obsolete/corrupt cache.
  }

  // Only the chat window needs an opaque loading canvas. Lens, translation,
  // and other overlay windows must retain their transparent backgrounds.
  if (/^#chat(?:[/-]|$)/.test(window.location.hash)) {
    root.dataset.themeBooting = 'true'
    root.style.backgroundColor = palette ? palette['--theme-surface'] : 'var(--theme-surface, #fdfcfa)'
  }
})()
