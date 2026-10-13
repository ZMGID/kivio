import { readFile, writeFile, mkdir } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

export const MODEL_DATABASE_REPO = 'ZMGID/kivio-models'
const MAX_BYTES = 4 * 1024 * 1024
export function versionKey(version) {
  if (typeof version !== 'string' || !/^\d{4}-\d{2}-\d{2}\.\d+$/.test(version)) throw new Error('Expected model database version YYYY-MM-DD.N')
  const parts = version.split(/[-.]/).map(Number)
  if (parts.some(n => !Number.isInteger(n) || n > 4294967295) || parts[1] < 1 || parts[1] > 12 || parts[2] < 1 || parts[2] > 31 || parts[3] < 1) throw new Error('Invalid model database version')
  return parts
}
export function compareVersions(a, b) {
  const left = versionKey(a), right = versionKey(b)
  for (let i = 0; i < left.length; i++) if (left[i] !== right[i]) return Math.sign(left[i] - right[i])
  return 0
}
export function validateDatabase(text) {
  if (Buffer.byteLength(text) > MAX_BYTES) throw new Error('Model database is too large')
  const data = JSON.parse(text)
  const object = value => value !== null && typeof value === 'object' && !Array.isArray(value)
  if (!object(data) || data._meta?.schemaVersion !== 1) throw new Error('Unsupported model database schema')
  versionKey(data._meta.version)
  if (Object.keys(data).length < 2) throw new Error('Model database is empty')
  for (const [id, entry] of Object.entries(data)) {
    if (id === '_meta') continue
    const invalid = () => { throw new Error(`Invalid model database entry: ${id}`) }
    if (!id || id.trim() !== id || id.toLowerCase() !== id || !object(entry) || typeof entry.displayName !== 'string' || !entry.displayName.trim()) invalid()
    for (const key of ['contextWindow', 'maxOutput', 'dimensions']) {
      const n = entry[key]
      if ((key === 'contextWindow' || n !== undefined) && (!Number.isInteger(n) || n < 0 || n > 4294967295)) invalid()
    }
    if (!object(entry.capabilities) || Object.values(entry.capabilities).some(v => typeof v !== 'boolean')) invalid()
    if (entry.pricing !== undefined && (!object(entry.pricing) || Object.values(entry.pricing).some(n => typeof n !== 'number' || !Number.isFinite(n) || n < 0))) invalid()
    if (entry.reasoningEfforts !== undefined && (!Array.isArray(entry.reasoningEfforts) || entry.reasoningEfforts.some(l => !['minimal', 'low', 'medium', 'high', 'xhigh', 'max'].includes(l)))) invalid()
    for (const key of ['thinkingOff', 'multilingual']) if (entry[key] !== undefined && typeof entry[key] !== 'boolean') invalid()
    if (entry.temperature !== undefined && (typeof entry.temperature !== 'number' || !Number.isFinite(entry.temperature) || entry.temperature < 0 || entry.temperature > 2)) invalid()
  }
  return data
}

async function fetchText(url, headers = {}) {
  const response = await fetch(url, { headers, signal: AbortSignal.timeout(30_000) })
  if (!response.ok) throw new Error(`${url}: HTTP ${response.status}`)
  let size = 0
  const chunks = []
  for await (const chunk of response.body) {
    size += chunk.length
    if (size > MAX_BYTES) throw new Error('Response is too large')
    chunks.push(chunk)
  }
  return Buffer.concat(chunks).toString('utf8')
}

async function main(args) {
  if (args[0] === '--check') {
    const data = validateDatabase(await readFile(args[1] ?? 'src/data/modelDatabase.json', 'utf8'))
    if (args[2]) {
      const previous = validateDatabase(await readFile(args[2], 'utf8'))
      if (compareVersions(data._meta.version, previous._meta.version) <= 0) throw new Error('Every published update needs a higher version')
    }
    console.log(`Validated ${data._meta.version}: ${Object.keys(data).length - 1} models`)
    return
  }
  const option = name => { const i = args.indexOf(name); return i < 0 ? undefined : args[i + 1] }
  const output = option('--output') ?? 'src/data/modelDatabase.json'
  let ref = option('--ref')
  if (!ref) {
    const headers = process.env.GITHUB_TOKEN ? { Authorization: `Bearer ${process.env.GITHUB_TOKEN}` } : {}
    const commit = JSON.parse(await fetchText(`https://api.github.com/repos/${MODEL_DATABASE_REPO}/commits/main`, headers))
    ref = commit.sha
  }
  if (!/^[a-f0-9]{40}$/.test(ref ?? '')) throw new Error('Expected an immutable 40-character model database commit SHA')
  const url = `https://raw.githubusercontent.com/${MODEL_DATABASE_REPO}/${ref}/v1/modelDatabase.json`
  const text = await fetchText(url)
  const data = validateDatabase(text)
  // Never silently ship an older catalog than the source checkout already contains.
  let bundled
  try { bundled = validateDatabase(await readFile('src/data/modelDatabase.json', 'utf8')) }
  catch (error) { if (error.code !== 'ENOENT') throw error }
  if (bundled && compareVersions(data._meta.version, bundled._meta.version) < 0) throw new Error('Online model database is older than the bundled snapshot')
  await mkdir(dirname(resolve(output)), { recursive: true })
  await writeFile(output, text)
  await writeFile(`${output}.source.json`, JSON.stringify({ repository: MODEL_DATABASE_REPO, commit: ref, version: data._meta.version }, null, 2) + '\n')
  console.log(`Synced model database ${data._meta.version} from ${MODEL_DATABASE_REPO}@${ref}`)
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2)).catch(error => { console.error(error.message); process.exitCode = 1 })
}
