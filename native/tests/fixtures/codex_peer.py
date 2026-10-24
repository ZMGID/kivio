#!/usr/bin/env python3
"""A stdio peer exercising the host boundary without an upstream account."""
import json, os, sys

def emit(value):
    print(json.dumps(value), flush=True)

for line in sys.stdin:
    request = json.loads(line)
    with open(os.environ['KIVIO_CLI_TEST_LOG'], 'a') as log:
        log.write(json.dumps({'pid':os.getpid(), 'cwd':os.getcwd(), 'request':request}) + '\n')
    method, params = request.get('method'), request.get('params', {})
    if method == 'initialized':
        continue
    if method == 'initialize':
        emit({'id':request['id'], 'result':{'userAgent':'fixture'}})
    elif method in ('thread/start', 'thread/resume'):
        emit({'id':request['id'], 'result':{'thread':{'id':'native-thread'}, 'sandbox':{'type':'workspaceWrite','writableRoots':[os.getcwd()]}, 'model':'fixture'}})
    elif method == 'turn/start':
        emit({'id':request['id'], 'result':{'turn':{'id':'turn-1'}}})
        emit({'method':'item/agentMessage/delta','params':{'threadId':'native-thread','turnId':'turn-1','itemId':'answer','delta':'partial'}})
        prompt = params['input'][0]['text']
        if 'WAIT_FOR_CANCEL' in prompt:
            continue
        emit({'id':'approval-1','method':'item/commandExecution/requestApproval',
              'params':{'threadId':'native-thread','itemId':'restricted-command','command':'restricted operation'}})
        decision = json.loads(sys.stdin.readline())
        with open(os.environ['KIVIO_CLI_TEST_LOG'], 'a') as log:
            log.write(json.dumps({'pid':os.getpid(), 'cwd':os.getcwd(), 'request':decision}) + '\n')
        assert decision.get('result', {}).get('decision') == 'decline', decision
        tool = {'type':'commandExecution','id':'tool-1','command':'cat attachment.txt','status':'inProgress'}
        emit({'method':'item/started','params':{'threadId':'native-thread','item':tool}})
        path = next((line[6:] for line in prompt.splitlines() if line.startswith('Path: ')), None)
        content = open(path).read() if path else 'no attachment'
        tool.update(status='completed', aggregatedOutput=content, exitCode=0)
        emit({'method':'item/completed','params':{'threadId':'native-thread','item':tool}})
        emit({'method':'item/completed','params':{'threadId':'native-thread','item':{'type':'agentMessage','id':'answer','text':'final corrected answer'}}})
        emit({'method':'turn/completed','params':{'threadId':'native-thread','turn':{'id':'turn-1','status':'completed'}}})
    elif method == 'turn/interrupt':
        emit({'id':request['id'], 'result':{}})
        emit({'method':'turn/completed','params':{'threadId':'native-thread','turn':{'id':'turn-1','status':'interrupted'}}})
    else:
        emit({'id':request.get('id'), 'error':{'code':-32601,'message':'unsupported fixture request'}})
