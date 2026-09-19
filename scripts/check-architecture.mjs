/* global process */
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import ts from 'typescript'

const DEFAULT_CONFIG = JSON.parse(fs.readFileSync(
  path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', 'architecture-boundaries.json'), 'utf8',
))

function matchesPath(pattern, file) {
  return pattern.endsWith('/**')
    ? file.startsWith(pattern.slice(0, -2))
    : file === pattern
}

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
  const record = (specifier, type, node) => found.push({
    specifier, type,
    line: source.getLineAndCharacterOfPosition(node.getStart(source)).line + 1,
  })
  function visit(node) {
    if ((ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) && node.moduleSpecifier && ts.isStringLiteral(node.moduleSpecifier)) {
      const typeOnly = ts.isImportDeclaration(node)
        ? node.importClause?.isTypeOnly || (node.importClause?.namedBindings && ts.isNamedImports(node.importClause.namedBindings)
          && node.importClause.namedBindings.elements.length > 0
          && node.importClause.namedBindings.elements.every((element) => element.isTypeOnly))
        : node.isTypeOnly || (node.exportClause && ts.isNamedExports(node.exportClause)
          && node.exportClause.elements.length > 0
          && node.exportClause.elements.every((element) => element.isTypeOnly))
      record(node.moduleSpecifier.text, typeOnly ? 'type-only' : ts.isExportDeclaration(node) ? 're-export' : 'runtime', node)
    }
    if (ts.isCallExpression(node) && node.expression.kind === ts.SyntaxKind.ImportKeyword && node.arguments.length === 1 && ts.isStringLiteral(node.arguments[0])) {
      record(node.arguments[0].text, 'dynamic', node)
    }
    if (ts.isImportTypeNode(node) && ts.isLiteralTypeNode(node.argument) && ts.isStringLiteral(node.argument.literal)) {
      record(node.argument.literal.text, 'type-only', node)
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

export function moduleInfo(root, file, config = DEFAULT_CONFIG) {
  const rel = relativeFile(root, file)
  if (!rel.startsWith('src/')) return null
  const matches = (config.modules ?? []).filter((module) => module.paths?.some((pattern) => matchesPath(pattern, rel)))
  if (matches.length > 1) throw new Error(`Ambiguous Module mapping for ${rel}: ${matches.map((module) => module.name).join(', ')}`)
  if (matches.length === 0) return null
  const module = matches[0]
  return {
    name: module.name,
    role: module.role,
    public: rel.startsWith(`src/${module.name}/public/`),
  }
}

/** Every resolved local TS import/re-export/dynamic-literal edge, including same-module edges. */
function collectDetailedEdges(root) {
  const src = path.join(root, 'src')
  const options = compilerOptions(root)
  const edges = new Map()
  for (const source of walk(src)) {
    for (const { specifier, type, line } of moduleSpecifiers(source)) {
      const resolved = ts.resolveModuleName(specifier, source, options, ts.sys).resolvedModule?.resolvedFileName
      if (!resolved || resolved.includes('/node_modules/') || resolved.includes('\\node_modules\\')) continue
      const target = path.resolve(resolved.replace(/\.d\.[cm]?ts$/, '.ts'))
      if (!target.startsWith(`${src}${path.sep}`)) continue
      const edge = {
        source: relativeFile(root, source),
        target: relativeFile(root, target),
        type,
        line,
        specifier,
      }
      edges.set(`${edge.source}:${line} -> ${edge.target} (${type})`, edge)
    }
  }
  return [...edges.values()].sort((a, b) => `${a.source}:${a.target}:${a.type}`.localeCompare(`${b.source}:${b.target}:${b.type}`))
}

export function collectDependencyEdges(root) {
  const edges = new Map()
  for (const { source, target } of collectDetailedEdges(root)) edges.set(`${source} -> ${target}`, { source, target })
  return [...edges.values()]
}

export function collectCrossDomainEdges(root, config = DEFAULT_CONFIG) {
  return collectDependencyEdges(root).filter((edge) => {
    const source = moduleInfo(root, path.join(root, edge.source), config)
    const target = moduleInfo(root, path.join(root, edge.target), config)
    return source && target && source.name !== target.name
  })
}

export function isPublicInterfaceEdge(edge) {
  return /^src\/[^/]+\/public\/[^/]+\.[cm]?[jt]sx?$/.test(edge.target)
}

function directViolation(root, edge, config) {
  const source = moduleInfo(root, path.join(root, edge.source), config)
  const target = moduleInfo(root, path.join(root, edge.target), config)
  if (!source || !target || source.name === target.name) return null
  if (source.role === 'composition' && config.modules.some((module) =>
    module.name === source.name && module.paths.includes(edge.source))) return null

  const targetIsFeature = target.role === 'feature'
  if (source.role === 'adapter' && targetIsFeature) return 'adapter_to_feature'
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

function crossFeatureCycles(root, edges, config) {
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
        .map((file) => moduleInfo(root, path.join(root, file), config))
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

function moduleCycles(root, edges, config) {
  const byPair = new Map()
  const adjacency = new Map()
  const nodes = new Set()
  for (const edge of edges) {
    const source = moduleInfo(root, path.join(root, edge.source), config)
    const target = moduleInfo(root, path.join(root, edge.target), config)
    if (!source || !target || source.name === target.name) continue
    nodes.add(source.name)
    nodes.add(target.name)
    const pair = `${source.name} -> ${target.name}`
    if (!byPair.has(pair)) byPair.set(pair, edge)
    const targets = adjacency.get(source.name) ?? new Set()
    targets.add(target.name)
    adjacency.set(source.name, targets)
  }
  const witnessFor = (members) => {
    const allowed = new Set(members)
    for (const start of members) {
      const visit = (current, visited, route) => {
        for (const next of [...(adjacency.get(current) ?? [])].filter((value) => allowed.has(value)).sort()) {
          if (next === start && route.length > 1) return [...route, start]
          if (visited.has(next)) continue
          const result = visit(next, new Set([...visited, next]), [...route, next])
          if (result) return result
        }
        return null
      }
      const route = visit(start, new Set([start]), [start])
      if (route) return {
        cycle: route,
        witness: route.slice(0, -1).map((module, index) => byPair.get(`${module} -> ${route[index + 1]}`)),
      }
    }
    throw new Error(`No witness for Module SCC: ${members.join(', ')}`)
  }
  return stronglyConnectedComponents(nodes, adjacency)
    .filter((members) => members.length > 1)
    .map((modules) => ({ kind: 'module_cycle', modules, ...witnessFor(modules) }))
}

function violationKey(violation) {
  if (violation.kind === 'module_cycle') return `module_cycle:${violation.modules.join(',')}`
  return `${violation.kind ?? 'deep_feature_import'}:${violation.source} -> ${violation.target}`
}

export function newViolations(root, config) {
  const policy = { ...DEFAULT_CONFIG, ...config }
  const baseline = new Set((policy.existingViolations ?? []).map(violationKey))
  const edges = collectDependencyEdges(root)
  const detailedEdges = collectDetailedEdges(root)
  const mappedPaths = new Set([...walk(path.join(root, 'src')).map((file) => relativeFile(root, file)), ...edges.flatMap((edge) => [edge.source, edge.target])])
  const unknown = [...mappedPaths].filter((file) => !moduleInfo(root, path.join(root, file), policy))
    .map((file) => ({ kind: 'unknown_module_path', source: file, target: file }))
  const direct = edges.flatMap((edge) => {
    const kind = directViolation(root, edge, policy)
    return kind ? [{ kind, ...edge }] : []
  })
  return [...unknown, ...direct, ...crossFeatureCycles(root, edges, policy), ...moduleCycles(root, detailedEdges, policy)]
    .filter((violation) => !baseline.has(violationKey(violation)))
    .sort((a, b) => violationKey(a).localeCompare(violationKey(b)))
}

export function formatViolation(violation) {
  if (violation.kind === 'module_cycle') {
    return [
      `[module_cycle] ${violation.cycle.join(' -> ')} (SCC: ${violation.modules.join(', ')})`,
      ...violation.witness.map((edge) =>
        `    [${edge.type}] ${edge.source}:${edge.line} -> ${edge.target} (${edge.specifier})`),
    ].join('\n')
  }
  return `[${violation.kind}] ${violation.source} -> ${violation.target}${violation.cycle
    ? `\n    ${violation.cycle.join(' -> ')}` : ''}`
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
    for (const violation of violations) console.error(`  ${formatViolation(violation)}`)
    process.exit(1)
  }

  console.log(`Architecture boundaries passed (${config.existingViolations.length} temporary exceptions, no new violations).`)
}
