import { useState, useEffect, useRef, type CSSProperties } from 'react'
import { Settings as SettingsIcon } from 'lucide-react'
import Settings from './Settings'
import ScreenshotResult from './ScreenshotResult'
import ScreenshotExplain from './ScreenshotExplain'
import './index.css'

type AppRegionStyle = CSSProperties & { WebkitAppRegion?: 'drag' | 'no-drag' }
const dragStyle: AppRegionStyle = { WebkitAppRegion: 'drag' }
const noDragStyle: AppRegionStyle = { WebkitAppRegion: 'no-drag' }

function Translator() {
  const [input, setInput] = useState('')
  const [result, setResult] = useState('')
  const [loading, setLoading] = useState(false)
  const [showSettings, setShowSettings] = useState(false)
  const [themeMode, setThemeMode] = useState<'system' | 'light' | 'dark'>('system')
  const resultRef = useRef<HTMLDivElement>(null)
  const inputRef = useRef<HTMLInputElement>(null)


  // Load Theme Preference and Apply
  const applyTheme = async () => {
    if (!window.api) return;
    const settings = await window.api.getSettings();

    // Set Theme
    const mode = settings.theme || 'system';
    setThemeMode(mode);

    const isDark = mode === 'dark' || (mode === 'system' && window.matchMedia('(prefers-color-scheme: dark)').matches);
    if (isDark) {
      document.documentElement.classList.add('dark');
    } else {
      document.documentElement.classList.remove('dark');
    }
  }

  useEffect(() => {
    applyTheme();
    // Listen for system theme changes if mode is system
    const mq = window.matchMedia('(prefers-color-scheme: dark)');
    const changeHandler = () => {
      if (themeMode === 'system') applyTheme();
    };
    mq.addEventListener('change', changeHandler);
    return () => mq.removeEventListener('change', changeHandler);
  }, [themeMode])

  // Manage window size based on view
  useEffect(() => {
    if (window.api) {
      if (showSettings) {
        window.api.resizeWindow(400, 520)
      } else {
        window.api.resizeWindow(360, 120)
      }
    }
  }, [showSettings])

  // Listen for Tray "Settings" click
  useEffect(() => {
    if (window.api) {
      const removeListener = window.api.onOpenSettings(() => {
        setShowSettings(true);
      });

      return () => {
        removeListener?.();
      };
    }
  }, [])


  // Debounce translation (only if not in settings)
  useEffect(() => {
    if (showSettings) return;

    const timer = setTimeout(async () => {
      if (input.trim()) {
        setLoading(true)
        try {
          if (!window.api) return
          const translated = await window.api.translateText(input);
          setResult(translated);
        } catch (error) {
          setResult('Error')
        } finally {
          setLoading(false)
        }
      } else {
        setResult('')
      }
    }, 600)

    return () => clearTimeout(timer)
  }, [input, showSettings])

  // Auto-scroll translation result to bottom when it updates
  useEffect(() => {
    if (resultRef.current) {
      resultRef.current.scrollTop = resultRef.current.scrollHeight;
    }
  }, [result])

  // Auto-scroll input to show cursor (keep right side visible)
  useEffect(() => {
    if (inputRef.current) {
      inputRef.current.scrollLeft = inputRef.current.scrollWidth;
    }
  }, [input])

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (showSettings) return; // Don't handle shortcuts in settings mode

    if (e.key === 'Enter') {
      const textToCommit = result || input;
      if (window.api) {
        window.api.commitTranslation(textToCommit)
        setInput('')
        setResult('')
      }
    } else if (e.key === 'Escape') {
      if (window.api) {
        window.api.closeWindow()
      }
    }
  }

  // Settings View
  if (showSettings) {
    const handleCloseSettings = () => {
      setShowSettings(false);
      // Hide window after closing settings (useful when opened from tray)
      if (window.api) {
        window.api.hideWindow();
      }
    };

    return (
      <div className="h-screen w-screen bg-white dark:bg-gray-900 rounded-lg border border-gray-200 dark:border-gray-700 shadow-xl overflow-hidden">
        <Settings onClose={handleCloseSettings} onSettingsChange={applyTheme} />
      </div>
    )
  }

  // Translation View
  return (
    <div className="h-screen w-screen flex flex-col p-2 bg-white/95 dark:bg-gray-900/95 rounded-lg border border-gray-200 dark:border-gray-700 shadow-xl backdrop-blur-sm select-none overflow-hidden relative group"
      style={dragStyle}>

      {/* Settings Toggle (Hidden by default, shows on hover) */}
      <button
        onClick={() => setShowSettings(true)}
        className="absolute top-2 right-2 p-1 text-gray-300 hover:text-gray-500 rounded no-drag opacity-0 group-hover:opacity-100 transition-opacity"
        style={noDragStyle}
      >
        <SettingsIcon size={14} />
      </button>

      {(result || loading) && (
        <div
          ref={resultRef}
          className="w-full mb-1 px-2 py-1 pr-8 bg-blue-50/50 dark:bg-blue-900/30 rounded text-base text-gray-800 dark:text-gray-200 font-medium select-text no-drag max-h-24 overflow-y-auto"
          style={noDragStyle}>
          {loading ? <span className="text-gray-400 text-sm">Translating...</span> : result}
        </div>
      )}

      <div className="flex items-center w-full">
        <input
          ref={inputRef}
          autoFocus
          className="w-full px-2 py-1 pr-8 bg-transparent text-lg text-gray-900 dark:text-white placeholder-gray-400 focus:outline-none no-drag"
          style={noDragStyle}
          placeholder="Translation input..."
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={handleKeyDown}
        />
      </div>
    </div>
  )
}

function App() {
  // Check URL mode for screenshot result or explain
  const urlParams = new URLSearchParams(window.location.search)
  const hash = window.location.hash.replace('#', '')
  const mode = urlParams.get('mode') || hash.split('?')[0]

  if (mode === 'screenshot') return <ScreenshotResult />
  if (mode === 'explain') return <ScreenshotExplain />
  return <Translator />
}

export default App
