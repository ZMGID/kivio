import ast
import asyncio
import concurrent.futures
import contextvars
import inspect
import json
import sys
import threading
import traceback

wire = sys.stdout
write_lock = threading.Lock()
state_lock = threading.Lock()
cell_id = contextvars.ContextVar('cell_id', default=None)
pending = {}
tools = {}
sequence = 0

def send(value):
    with write_lock:
        wire.write(json.dumps(value, ensure_ascii=False, default=repr) + '\n')
        wire.flush()

class Output:
    def write(self, text):
        if text:
            send({'type': 'output', 'cell': cell_id.get(), 'text': text})
        return len(text)
    def flush(self):
        pass

sys.stdout = Output()
sys.stderr = Output()

async def host(method, args):
    global sequence
    with state_lock:
        sequence += 1
        request_id = f'host-{sequence}'
        future = concurrent.futures.Future()
        pending[request_id] = future
    send({'type': 'host', 'id': request_id, 'cell': cell_id.get(), 'method': method, 'args': args})
    return await asyncio.wrap_future(future)

def tool(fn=None, *, name=None, description='', parameters=None):
    def register(function):
        key = name or function.__name__
        if not key.replace('_', '').replace('-', '').isalnum():
            raise ValueError('A tool needs a valid name')
        tools[key] = {'fn': function, 'name': key, 'description': description, 'parameters': parameters or {'type': 'object', 'properties': {}}, 'language': 'py'}
        return function
    return register(fn) if fn else register

async def agent(request):
    return await host('agent', request)

class Pool:
    def __init__(self, identity):
        self.id = identity
    async def push(self, *items):
        values = [value for item in items for value in (item if isinstance(item, list) else [item])]
        return await host('pool.push', {'id': self.id, 'items': values})
    async def peek(self):
        return await host('pool.peek', {'id': self.id})
    async def status(self):
        return await host('pool.status', {'id': self.id})
    async def drain(self):
        return await host('pool.drain', {'id': self.id})
    async def close(self):
        return await host('pool.close', {'id': self.id})

async def workpool(options):
    return Pool((await host('pool.create', options))['id'])

scope = {'__name__': '__kivio_eval__', 'tool': tool, 'agent': agent, 'workpool': workpool, 'undefine_tool': lambda name: tools.pop(name, None) is not None}

def execute(request):
    token = cell_id.set(request.get('id'))
    try:
        operation = request.get('op')
        if operation == 'eval':
            tree = ast.parse(request['code'], '<subagent-eval>', mode='exec')
            final = tree.body.pop() if tree.body and isinstance(tree.body[-1], ast.Expr) else None
            code = compile(tree, '<subagent-eval>', 'exec', flags=ast.PyCF_ALLOW_TOP_LEVEL_AWAIT)
            value = eval(code, scope)
            if inspect.isawaitable(value):
                asyncio.run(value)
            if final:
                expression = ast.Expression(final.value)
                ast.fix_missing_locations(expression)
                value = eval(compile(expression, '<subagent-eval>', 'eval', flags=ast.PyCF_ALLOW_TOP_LEVEL_AWAIT), scope)
            else:
                value = None
        elif operation == 'describe':
            names = request.get('names', [])
            value = [{key: val for key, val in descriptor.items() if key != 'fn'} for name, descriptor in tools.items() if not names or name in names]
        elif operation == 'invoke':
            entry = tools.get(request['name'])
            if not entry:
                raise ValueError(f"Tool {request['name']} is unavailable")
            value = entry['fn'](**request.get('arguments', {}))
        else:
            raise ValueError('Unknown kernel operation')
        if inspect.isawaitable(value):
            value = asyncio.run(value)
        send({'type': 'result', 'id': request['id'], 'ok': True, 'value': value})
    except BaseException as error:
        send({'type': 'result', 'id': request['id'], 'ok': False, 'error': ''.join(traceback.format_exception_only(type(error), error)).strip()})
    finally:
        cell_id.reset(token)

cells = concurrent.futures.ThreadPoolExecutor(max_workers=1)
calls = concurrent.futures.ThreadPoolExecutor(max_workers=4)
for line in sys.stdin:
    try:
        request = json.loads(line)
    except json.JSONDecodeError:
        continue
    if request.get('type') == 'host-result':
        with state_lock:
            future = pending.pop(request.get('id'), None)
        if future:
            if request.get('ok'):
                future.set_result(request.get('value'))
            else:
                future.set_exception(RuntimeError(request.get('error', 'Host call failed')))
    else:
        (cells if request.get('op') == 'eval' else calls).submit(execute, request)
