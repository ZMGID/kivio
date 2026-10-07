import { createContext } from 'react'

/** Cached conversation elements must observe visibility even when their props are retained. */
export const ChatSurfaceActivityContext = createContext(true)
