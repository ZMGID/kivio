import { pathFromHash } from './routeCodec'

/** Browser adapter for the pure route codec. */
export function hashPath(): string {
  return pathFromHash(window.location.hash)
}
