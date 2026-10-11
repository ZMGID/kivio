/// Vite 客户端环境类型声明
/// 引入 Vite 提供的客户端类型定义（如 import.meta.env、模块热更新等）
/// <reference types="vite/client" />

// Generated at Vite load time from the authoritative Rust-exported schemas.
declare module 'virtual:chat-protocol-validators' {
  import type { ValidateFunction } from 'ajv'
  export const validateEvent: ValidateFunction
  export const validateSync: ValidateFunction
}
