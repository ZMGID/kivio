const fs = require('node:fs');
const [log, mode = 'push'] = process.argv.slice(2);
if (mode === 'tree') {
  const child = require('node:child_process').spawn(process.execPath, ['-e', 'setInterval(() => {}, 1000)'], { stdio: 'ignore' });
  fs.writeFileSync(log + '.child', String(child.pid));
}
let buffer = Buffer.alloc(0);
let first;
let initialized = false;

function send(value) {
  const bytes = Buffer.from(JSON.stringify({ jsonrpc: '2.0', ...value }));
  process.stdout.write(`Content-Length: ${bytes.length}\r\n\r\n`);
  process.stdout.write(bytes);
}

function diagnostic(message) {
  return { range: { start: { line: 0, character: 0 }, end: { line: 0, character: 1 } }, severity: 1, message };
}

function publish(uri, version, message) {
  send({ method: 'textDocument/publishDiagnostics', params: { uri, ...(version === undefined ? {} : { version }), diagnostics: [diagnostic(message)] } });
}

function handle(message) {
  fs.appendFileSync(log, JSON.stringify(message) + '\n');
  if (message.method === 'initialize') {
    if (mode === 'crash') process.exit(4);
    if (mode === 'hang') return;
    send({ id: message.id, result: { capabilities: mode === 'pull' ? { diagnosticProvider: { interFileDependencies: false, workspaceDiagnostics: false } } : {} } });
  } else if (message.method === 'initialized') {
    initialized = true;
  } else if (message.method === 'textDocument/didOpen' || message.method === 'textDocument/didChange') {
    if (!initialized) process.exit(5);
    const { uri, version } = message.params.textDocument;
    if (!first) first = { uri, version };
    if (mode === 'push' || mode === 'tree') {
      if (version === 1) publish(uri, version, 'initial');
      else {
        publish(first.uri, first.version, 'late old version');
        publish(uri, version, 'current version');
      }
    } else if (mode === 'versionless') {
      publish(uri, undefined, 'versionless');
    }
  } else if (message.method === 'textDocument/diagnostic') {
    if (first.request === undefined) {
      first.request = message.id;
    } else {
      send({ id: first.request, result: { kind: 'full', items: [diagnostic('late old pull')] } });
      send({ id: message.id, result: { kind: 'full', items: [diagnostic('current pull')] } });
    }
  } else if (message.method === 'shutdown') {
    send({ id: message.id, result: null });
  } else if (message.method === 'exit') {
    process.exit(0);
  }
}

process.stdin.on('data', chunk => {
  buffer = Buffer.concat([buffer, chunk]);
  while (true) {
    const end = buffer.indexOf('\r\n\r\n');
    if (end < 0) return;
    const length = Number(/Content-Length: (\d+)/i.exec(buffer.subarray(0, end).toString())[1]);
    if (buffer.length < end + 4 + length) return;
    const message = JSON.parse(buffer.subarray(end + 4, end + 4 + length));
    buffer = buffer.subarray(end + 4 + length);
    handle(message);
  }
});
