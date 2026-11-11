import { createContext } from 'react'

/** 缓存页的可见状态也传给挂在 document.body 的浮层；独立窗口默认可见。 */
export const ChatRouteActiveContext = createContext(true)
