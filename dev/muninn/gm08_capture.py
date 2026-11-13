"""Frozen GM08 operations shared by reference and product; no API value uses graph data."""
from copy import deepcopy
from datetime import datetime, timezone
import json
from pathlib import Path

FIXTURE = Path(__file__).with_name('fixtures') / 'gm08-v1.json'


def frozen_fixture():
    return json.loads(FIXTURE.read_text())


async def capture_gm08(adapter, fixture=None):
    """Capture raw writes, independent reads/graph, then reads on a new connection.

    The adapter owns seed/schema setup and reconnect. No fallback or archived
    expected document is accepted. A missing operation aborts, never becomes [].
    """
    frozen = frozen_fixture()
    if fixture is None:
        fixture = frozen
    if fixture != frozen:
        raise ValueError('fixture differs from frozen GM08 inputs')
    start = datetime.now(timezone.utc).isoformat()
    bindings, writes = {}, []
    await adapter.seed(fixture)

    async def write(operation, **arguments):
        result = await adapter.call(operation, **deepcopy(arguments))
        writes.append({'operation': operation, 'arguments': deepcopy(arguments), 'result': deepcopy(result)})
        return result

    for task in fixture['tasks']:
        key = task['key']
        trace = await write('start_trace', session_id=fixture['session_id'], task=task['task'],
                            generate_embedding=True, triggered_by_message_id=fixture['message_id'])
        bindings[key] = trace['id']
        for number, step in enumerate(task['steps'], 1):
            created = await write('add_step', trace_id=trace['id'], thought=None,
                                  action=step['action'], observation=step['observation'],
                                  generate_embedding=True)
            bindings[f'{key}.step{number}'] = created['id']
            call = await write('record_tool_call', step_id=created['id'], tool_name=fixture['tool_name'],
                               arguments=step['arguments'], result=step['result'], status=step['status'],
                               duration_ms=step['duration_ms'], error=step['error'],
                               message_id=fixture['message_id'], touched_entities=[{'id': fixture['entity_id']}])
            bindings[f'{key}.call{number}'] = call['id']
        outcome = deepcopy(task['outcome'])
        if isinstance(outcome, dict):
            outcome['related_entities'] = [{'id': fixture['entity_id']}]
        await write('complete_trace', trace_id=trace['id'], outcome=outcome,
                    success=task['success'], generate_step_embeddings=True)

    async def reads():
        traces = [await adapter.call('get_trace_with_steps', trace_id=bindings[task['key']])
                  for task in fixture['tasks']]
        lists = {}
        controls = {
            'ordered': {'order_by': 'started_at', 'order_dir': 'asc'},
            'completed': {'order_by': 'completed_at', 'order_dir': 'desc'},
            'successful': {'success_only': True},
            'failed': {'success_only': False},
            'page': {'limit': 1, 'offset': 1, 'order_dir': 'asc'},
            'supplied_dates': {'since': '2020-01-01T01:00:00+01:00', 'until': '2030-01-01T00:00:00Z'},
        }
        for name, arguments in controls.items():
            lists[name] = await adapter.call('list_traces', session_id=fixture['session_id'], **arguments)
        searches = {}
        for search in fixture['searches']:
            arguments = {name: value for name, value in search.items() if name != 'key'}
            searches[search['key']] = await adapter.call('get_similar_traces', task=fixture['tasks'][0]['task'], **arguments)
        return {'traces': traces, 'stats': await adapter.call('get_tool_stats', tool_name=fixture['tool_name']),
                'lists': lists, 'searches': searches}

    api = await reads()
    api['writes'] = writes
    graph = await adapter.snapshot()
    await adapter.reconnect()
    after = {'api': await reads(), 'graph': await adapter.snapshot()}
    return {'schema': 'gm08-capture-v1', 'window': [start, datetime.now(timezone.utc).isoformat()],
            'bindings': bindings, 'api': api, 'graph': graph, 'reconnect': after}
