(function () {
        var ua = navigator.userAgent
        if (/Windows/i.test(ua)) document.documentElement.classList.add('windows')
        else if (/Mac|iPhone|iPad|iPod/i.test(ua)) document.documentElement.classList.add('macos')

        // 记录最近一次输入来源，供焦点环判断（见 shared-surfaces.css）。WebKit 对脚本聚焦
        // （确认框默认按钮、菜单关闭后还焦点）也会命中 :focus-visible，单靠伪类挡不住鼠标操作后的焦点环。
        var root = document.documentElement
        root.dataset.focusSource = 'pointer'
        document.addEventListener('pointerdown', function () {
          root.dataset.focusSource = 'pointer'
        }, true)
        document.addEventListener('keydown', function (e) {
          if (e.metaKey || e.ctrlKey || e.altKey) return
          root.dataset.focusSource = 'keyboard'
        }, true)
      })()
