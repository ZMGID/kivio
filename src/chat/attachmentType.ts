/** Matches the backend's video MIME allowlist. */
export function isVideoFile(name: string): boolean {
  return /\.(mp4|mpeg|mov|avi|flv|mpg|webm|wmv|3gp|3gpp)$/i.test(name)
}
