import { seedModelDatabase } from './modelMatching'

/**
 * Load the catalog shipped in the bundle. Only for runs without a backend
 * (browser preview): the desktop app receives its catalog from the backend
 * before mounting and never fetches this chunk.
 */
export async function loadBundledModelDatabase(): Promise<void> {
  const { default: database } = await import('./modelDatabase.json')
  seedModelDatabase(database)
}
