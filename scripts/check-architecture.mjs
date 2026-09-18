/* global process */
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import ts from 'typescript'

const MODULE_ROLES = new Map([
  ['chat', 'feature'],
  ['settings', 'feature'],
  ['lens', 'feature'],
  ['onboarding', 'feature'],
  ['api', 'adapter'],
  ['components', 'shared-ui'],
  ['data', 'foundation'],
  ['utils', 'foundation'],
  ['generated', 'foundation'],
  ['test', 'foundation'],
])

/** Root files which deliberately assemble features rather than belonging to one. */
export const COMPOSITION_ROOTS = new Set([
  'src/App.tsx',
  'src/Lens.tsx',
  'src/main.tsx',
])

function walk(directory) {
  return fs.readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const full = path.join(directory, entry.name)
    if (entry.isDirectory()) return walk(full)
    if (!/\.[cm]?tsx?$/.test(entry.name) || /\.(test|spec)\.[cm]?tsx?$/.test(entry.name)) return []
    return [full]
  })
}

function moduleSpecifiers(file) {
  const source = ts.createSourceFile(file, fs.readFileSync(file, 'utf8'), ts.ScriptTarget.Latest, true)
  const found = []
  function visit(node) {
    if ((ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) && node.moduleSpecifier && ts.isStringLiteral(node.moduleSpecifier)) {
      found.push(node.moduleSpecifier.text)
    }
    if (ts.isCallExpression(node) && node.expression.kind === ts.SyntaxKind.ImportKeyword && node.arguments.length === 1 && ts.isStringLiteral(node.arguments[0])) {
      found.push(node.arguments[0].text)
    }
    ts.forEachChild(node, visit)
  }
  visit(source)
  return found
}

function compilerOptions(root) {
  const configPath = path.join(root, 'tsconfig.json')
  if (!fs.existsSync(configPath)) return { moduleResolution: ts.ModuleResolutionKind.Bundler }
  const loaded = ts.readConfigFile(configPath, ts.sys.readFile)
  if (loaded.error) throw new Error(ts.flattenDiagnosticMessageText(loaded.error.messageText, '\n'))
  return ts.parseJsonConfigFileContent(loaded.config, ts.sys, root).options
}

function relativeFile(root, file) {
  return path.relative(root, file).replaceAll('\\', '/')
}

export function moduleInfo(root, file) {
  const rel = relativeFile(root, file)
  const parts = rel.split('/')
  if (parts[0] !== 'src') return null
  if (parts.length === 2) {
    return {
      name: 'root',
      role: COMPOSITION_ROOTS.has(rel) ? 'composition' : 'foundation',
      public: false,
    }
  }
  const name = parts[1]
  return {
    name,
    role: MODULE_ROLES.get(name) ?? 'feature',
    public: parts[2] === 'public',
  }
}

/** Every resolved local TS import/re-export/dynamic-literal edge, including same-module edges. */
export function collectDependencyEdges(root) {
  const src = path.join(root, 'src')
  const options = compilerOptions(root)
  const edges = new Map()
  for (const source of walk(src)) {
    for (const specifier of moduleSpecifiers(source)) {
      const resolved = ts.resolveModuleName(specifier, source, options, ts.sys).resolvedModule?.resolvedFileName
      if (!resolved || resolved.includes('/node_modules/') || resolved.includes('\\node_modules\\')) continue
      const target = path.resolve(resolved.replace(/\.d\.[cm]?ts$/, '.ts'))
      if (!target.startsWith(`${src}${path.sep}`)) continue
      const edge = {
        source: relativeFile(root, source),
        target: relativeFile(root, target),
      }
      edges.set(`${edge.source} -> ${edge.target}`, edge)
    }
  }
  return [...edges.values()].sort((a, b) => `${a.source}:${a.target}`.localeCompare(`${b.source}:${b.target}`))
}

export function collectCrossDomainEdges(root) {
  return collectDependencyEdges(root).filter((edge) => {
    const source = moduleInfo(root, path.join(root, edge.source))
    const target = moduleInfo(root, path.join(root, edge.target))
    return source && target && source.name !== target.name
  })
}

export function isPublicInterfaceEdge(edge) {
  return /^src\/[^/]+\/public\/[^/]+\.[cm]?[jt]sx?$/.test(edge.target)
}

function directViolation(root, edge) {
  const source = moduleInfo(root, path.join(root, edge.source))
  const target = moduleInfo(root, path.join(root, edge.target))
  if (!source || !target || source.name === target.name) return null
  if (source.role === 'composition' && COMPOSITION_ROOTS.has(edge.source)) return null

  const targetIsFeature = target.role === 'feature'
  if ((source.role === 'foundation' || source.role === 'shared-ui') && targetIsFeature) return 'shared_to_feature'
  if (source.public && targetIsFeature) return 'public_reverse_dependency'
  if (targetIsFeature && !isPublicInterfaceEdge(edge)) return 'deep_feature_import'
  return null
}

function stronglyConnectedComponents(nodes, adjacency) {
  let index = 0
  const indices = new Map()
  const lowLinks = new Map()
  const stack = []
  const onStack = new Set()
  const components = []

  function connect(node) {
    indices.set(node, index)
    lowLinks.set(node, index)
    index += 1
    stack.push(node)
    onStack.add(node)

    for (const next of adjacency.get(node) ?? []) {
      if (!indices.has(next)) {
        connect(next)
        lowLinks.set(node, Math.min(lowLinks.get(node), lowLinks.get(next)))
      } else if (onStack.has(next)) {
        lowLinks.set(node, Math.min(lowLinks.get(node), indices.get(next)))
      }
    }

    if (lowLinks.get(node) !== indices.get(node)) return
    const component = []
    while (stack.length > 0) {
      const member = stack.pop()
      onStack.delete(member)
      component.push(member)
      if (member === node) break
    }
    components.push(component.sort())
  }

  for (const node of [...nodes].sort()) if (!indices.has(node)) connect(node)
  return components
}

function crossFeatureCycles(root, edges) {
  const localFiles = new Set()
  const adjacency = new Map()
  for (const edge of edges) {
    localFiles.add(edge.source)
    localFiles.add(edge.target)
    const targets = adjacency.get(edge.source) ?? []
    targets.push(edge.target)
    adjacency.set(edge.source, targets)
  }

  return stronglyConnectedComponents(localFiles, adjacency)
    .filter((component) => component.length > 1)
    .filter((component) => {
      const featureDomains = component
        .map((file) => moduleInfo(root, path.join(root, file)))
        .filter((info) => info?.role === 'feature')
        .map((info) => info.name)
      return new Set(featureDomains).size > 1
    })
    .map((component) => ({
      kind: 'cross_feature_cycle',
      source: component[0],
      target: component[0],
      cycle: component,
    }))
}

function violationKey(violation) {
  return `${violation.kind ?? 'deep_feature_import'}:${violation.source} -> ${violation.target}`
}

export function newViolations(root, config) {
  const baseline = new Set((config.existingViolations ?? []).map(violationKey))
  const edges = collectDependencyEdges(root)
  const direct = edges.flatMap((edge) => {
    const kind = directViolation(root, edge)
    return kind ? [{ kind, ...edge }] : []
  })
  return [...direct, ...crossFeatureCycles(root, edges)]
    .filter((violation) => !baseline.has(violationKey(violation)))
    .sort((a, b) => violationKey(a).localeCompare(violationKey(b)))
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
  const configPath = path.join(root, 'architecture-boundaries.json')
  const config = JSON.parse(fs.readFileSync(configPath, 'utf8'))
  const violations = newViolations(root, config)

  if (process.argv.includes('--list')) {
    process.stdout.write(`${JSON.stringify(violations, null, 2)}\n`)
    process.exit(0)
  }

  if (violations.length > 0) {
    console.error('Architecture boundary violations:')
    for (const violation of violations) {
      console.error(`  [${violation.kind}] ${violation.source} -> ${violation.target}`)
      if (violation.cycle) console.error(`    ${violation.cycle.join(' -> ')}`)
    }
    process.exit(1)
  }

  console.log(`Architecture boundaries passed (${config.existingViolations.length} temporary exceptions, no new violations).`)
}
