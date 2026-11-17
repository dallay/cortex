"""GM08 exact, fail-closed comparison. Offline comparison is NOT a live verdict.

Only random UUID4 bijections, bounded generated time and JSON representation may
normalize. Ordered API lists stay ordered. Graph nodes/edges alone are unordered.
No GM05 deviations, field drops, graph-to-API repairs or numeric tolerance apply.
"""
import argparse
from copy import deepcopy
from datetime import datetime, timezone
import json
import math
from pathlib import Path
from uuid import UUID

FIXTURE = Path(__file__).with_name('fixtures') / 'gm08-v1.json'
TRACE = set('id session_id task steps outcome success started_at completed_at'.split())
STEP = set('id trace_id step_number thought action observation tool_calls'.split())
CALL = set('id tool_name arguments result status duration_ms error'.split())
STATS = set('name total_calls successful_calls failed_calls success_rate avg_duration_ms'.split())
# Raw native API fields remain compared when exposed; no missing/null defaulting.
EXTRA = set('created_at updated_at embedding metadata'.split())
GRAPH = {
    'ReasoningTrace': set('id session_id task task_embedding outcome success started_at completed_at metadata error_kind metrics_json user_identifier'.split()),
    'ReasoningStep': set('id step_number thought action observation embedding timestamp metadata'.split()),
    'ToolCall': set('id tool_name arguments result status duration_ms error timestamp'.split()),
    'Tool': set('name description created_at last_used_at total_calls successful_calls failed_calls total_duration_ms'.split()),
    'Message': set('id content role timestamp metadata'.split()),
    'Entity': set('id name type'.split()),
    'Conversation': set('id session_id owner_scope'.split()),
}
EDGES = {'HAS_STEP': {'order'}, 'USES_TOOL': set(), 'INSTANCE_OF': set(),
         'INITIATED_BY': set(), 'TRIGGERED_BY': set(), 'TOUCHED': {'recorded_at'},
         'HAS_MESSAGE': set(), 'MENTIONS': set()}
GENERATED = {'created_at', 'updated_at', 'timestamp', 'started_at', 'completed_at',
             'last_used_at', 'recorded_at'}


class ContractError(ValueError):
    pass


def fields(value, required, allowed, path):
    if not isinstance(value, dict):
        raise ContractError(f'{path}: object required')
    missing, unknown = required - value.keys(), value.keys() - allowed
    if missing or unknown:
        raise ContractError(f'{path}: missing={sorted(missing)} unknown={sorted(unknown)}')


def moment(value):
    if not isinstance(value, str):
        raise ContractError('timestamp: string required')
    parsed = datetime.fromisoformat(value.replace('Z', '+00:00'))
    if parsed.utcoffset() is None:
        raise ContractError('timestamp: timezone required')
    return parsed.astimezone(timezone.utc)


def encode(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'), allow_nan=False)


def _record(value, category, path):
    required = {'trace': TRACE, 'step': STEP, 'call': CALL, 'stats': STATS}[category]
    extra = EXTRA | ({'task_embedding'} if category == 'trace' else set())
    if category == 'call': extra |= {'step_id'}
    if category == 'stats': extra = {'description', 'last_used_at'}
    fields(value, required, required | extra, path)
    if category == 'trace':
        if value['success'] is not None and type(value['success']) is not bool:
            raise ContractError(path + '.success: boolean or null required')
        for name in ('session_id', 'task'):
            if not isinstance(value[name], str):
                raise ContractError(path + '.' + name + ': string required')
        if not isinstance(value['steps'], list):
            raise ContractError(path + '.steps: list required')
        numbers = [s.get('step_number') for s in value['steps']]
        if numbers != list(range(1, len(numbers) + 1)):
            raise ContractError(path + '.steps: ordered sequential steps required')
        for index, step in enumerate(value['steps']):
            _record(step, 'step', f'{path}.steps[{index}]')
            if step['trace_id'] != value['id']:
                raise ContractError(path + '.steps: inconsistent trace reference')
        if value['completed_at'] is not None and moment(value['completed_at']) < moment(value['started_at']):
            raise ContractError(path + ': completion precedes start')
    elif category == 'step':
        if type(value['step_number']) is not int:
            raise ContractError(path + '.step_number: integer required')
        if not isinstance(value['tool_calls'], list):
            raise ContractError(path + '.tool_calls: list required')
        for index, call in enumerate(value['tool_calls']):
            _record(call, 'call', f'{path}.tool_calls[{index}]')
            if 'step_id' in call and call['step_id'] != value['id']:
                raise ContractError(path + ': inconsistent step reference')
    elif category == 'call':
        if value['status'] not in {'pending', 'success', 'failure', 'error', 'timeout', 'cancelled'}:
            raise ContractError(path + '.status: unknown status')
        if value['duration_ms'] is not None and type(value['duration_ms']) is not int:
            raise ContractError(path + '.duration_ms: integer or null required')
        if not isinstance(value['arguments'], dict):
            raise ContractError(path + '.arguments: object required')
    elif category == 'stats':
        for name in ('total_calls', 'successful_calls', 'failed_calls'):
            if type(value[name]) is not int or value[name] < 0:
                raise ContractError(path + '.' + name + ': nonnegative integer required')
        for name in ('success_rate', 'avg_duration_ms'):
            if value[name] is not None and (type(value[name]) not in (int, float) or not math.isfinite(value[name])):
                raise ContractError(path + '.' + name + ': finite number required')


def _api(value, path):
    required = {'traces', 'stats', 'lists', 'searches'}
    if path == 'api':
        required.add('writes')
    fields(value, required, required | {'writes'}, path)
    if 'writes' in value:
        if not isinstance(value['writes'], list) or len(value['writes']) != 16:
            raise ContractError(path + '.writes: exactly sixteen writes required')
        for index, write in enumerate(value['writes']):
            location = f'{path}.writes[{index}]'
            fields(write, {'operation', 'arguments', 'result'}, {'operation', 'arguments', 'result'}, location)
            category = {'start_trace': 'trace', 'add_step': 'step', 'record_tool_call': 'call', 'complete_trace': 'trace'}.get(write['operation'])
            if category is None or not isinstance(write['arguments'], dict):
                raise ContractError(location + ': invalid operation or arguments')
            _record(write['result'], category, location + '.result')
    for name, category in (('traces', 'trace'), ('stats', 'stats')):
        if not isinstance(value[name], list):
            raise ContractError(path + '.' + name + ': list required')
        for index, item in enumerate(value[name]):
            _record(item, category, f'{path}.{name}[{index}]')
    for name in ('lists', 'searches'):
        if not isinstance(value[name], dict) or not value[name]:
            raise ContractError(path + '.' + name + ': nonempty operation map required')
        required_operations = ({'ordered', 'completed', 'successful', 'failed', 'page', 'supplied_dates'}
                               if name == 'lists' else {'all', 'successful', 'limit', 'threshold'})
        if set(value[name]) != required_operations:
            raise ContractError(path + '.' + name + ': missing or unknown operation')
        for operation, items in value[name].items():
            if not isinstance(items, list):
                raise ContractError(path + '.' + name + '.' + operation + ': list required')
            for index, item in enumerate(items):
                _record(item, 'trace', f'{path}.{name}.{operation}[{index}]')


def canonical_gm08(document):
    fields(document, {'schema', 'window', 'bindings', 'api', 'graph', 'reconnect'},
           {'schema', 'window', 'bindings', 'api', 'graph', 'reconnect'}, 'capture')
    if document['schema'] != 'gm08-capture-v1':
        raise ContractError('schema: unsupported capture')
    fixture = json.loads(FIXTURE.read_text())
    expected_tokens = {task['key'] + suffix for task in fixture['tasks']
                       for suffix in ('', '.step1', '.step2', '.step3', '.call1', '.call2', '.call3')}
    fields(document['bindings'], expected_tokens, expected_tokens, 'bindings')
    mapping = {}
    for token, identifier in document['bindings'].items():
        parsed = UUID(identifier)
        if parsed.version != 4 or str(parsed) != identifier or identifier in mapping:
            raise ContractError('bindings: canonical unique random UUID4 required')
        mapping[identifier] = '<' + token + '>'
    for name in ('entity_id', 'message_id', 'conversation_id'):
        if fixture[name] in mapping:
            raise ContractError('bindings: supplied fixture ID is not random')
    if not isinstance(document['window'], list) or len(document['window']) != 2:
        raise ContractError('window: two bounds required')
    start, end = map(moment, document['window'])
    if start > end:
        raise ContractError('window: reversed bounds')
    supplied = {fixture[name] for name in ('entity_id', 'message_id', 'conversation_id')}

    supplied_time_paths = set()

    def normalize(value, path='', key=''):
        if isinstance(value, dict):
            return {name: normalize(item, path + '.' + name, name) for name, item in value.items()}
        if isinstance(value, list):
            return [normalize(item, f'{path}[{index}]', key) for index, item in enumerate(value)]
        if type(value) is float and not math.isfinite(value):
            raise ContractError(path + ': nonfinite number')
        if isinstance(value, str):
            if key in GENERATED:
                parsed = moment(value)
                # The explicit message timestamp is a supplied input, not generated.
                if path in supplied_time_paths:
                    if parsed != moment(fixture['timestamp']):
                        raise ContractError(path + ': supplied timestamp differs')
                    return parsed.isoformat()
                if not start <= parsed <= end:
                    raise ContractError(path + ': outside capture window')
                return '<generated-time>'
            if key in {'since', 'until'}:
                return moment(value).isoformat()
            if key in {'metrics_json', 'arguments', 'result', 'metadata'}:
                return normalize(json.loads(value), path)
            if value in mapping:
                return mapping[value]
            if key in {'id', 'trace_id', 'step_id', 'key', 'source', 'target'}:
                if value not in supplied and value != 'Tool:' + fixture['tool_name']:
                    raise ContractError(path + ': undeclared identity')
        return value

    _api(document['api'], 'api')
    fields(document['reconnect'], {'api', 'graph'}, {'api', 'graph'}, 'reconnect')
    _api(document['reconnect']['api'], 'reconnect.api')
    result = {'api': normalize(document['api']), 'reconnect': {'api': normalize(document['reconnect']['api'])}}
    for graph, path in ((document['graph'], 'graph'), (document['reconnect']['graph'], 'reconnect.graph')):
        fields(graph, {'nodes', 'edges'}, {'nodes', 'edges'}, path)
        nodes, edges, keys = [], [], set()
        for index, node in enumerate(graph['nodes']):
            node_path = f'{path}.nodes[{index}]'
            fields(node, {'key', 'labels', 'properties'}, {'key', 'labels', 'properties'}, node_path)
            if not isinstance(node['labels'], list) or len(node['labels']) != 1 or node['labels'][0] not in GRAPH:
                raise ContractError(node_path + ': unknown label')
            label = node['labels'][0]
            fields(node['properties'], {'name'} if label == 'Tool' else {'id'}, GRAPH[label], node_path + '.properties')
            if node['key'] in keys:
                raise ContractError(node_path + ': duplicate node')
            if node['key'] != (('Tool:' + node['properties']['name']) if label == 'Tool' else node['properties']['id']):
                raise ContractError(node_path + ': inconsistent key')
            keys.add(node['key'])
            if label == 'Message' and node['key'] == fixture['message_id']:
                if 'timestamp' not in node['properties']:
                    raise ContractError(node_path + ': supplied timestamp absent')
                supplied_time_paths.add(node_path + '.properties.timestamp')
            properties = node['properties']
            if label == 'ReasoningTrace' and properties.get('completed_at') is not None:
                if moment(properties['completed_at']) < moment(properties['started_at']):
                    raise ContractError(node_path + ': completion precedes start')
            nodes.append(normalize(node, node_path))
        edge_set = set()
        for index, edge in enumerate(graph['edges']):
            edge_path = f'{path}.edges[{index}]'
            fields(edge, {'source', 'target', 'type', 'properties'}, {'source', 'target', 'type', 'properties'}, edge_path)
            if edge['type'] not in EDGES or edge['source'] not in keys or edge['target'] not in keys:
                raise ContractError(edge_path + ': unknown/dangling edge')
            fields(edge['properties'], set(), EDGES[edge['type']], edge_path + '.properties')
            edge_set.add((edge['source'], edge['type'], edge['target']))
            edges.append(normalize(edge, edge_path))
        # Independent fixed-fixture invariants catch equal but incomplete captures.
        for task in fixture['tasks']:
            tid = document['bindings'][task['key']]
            required = [(tid, 'INITIATED_BY', fixture['message_id'])]
            for number in range(1, 4):
                sid = document['bindings'][f"{task['key']}.step{number}"]
                cid = document['bindings'][f"{task['key']}.call{number}"]
                required.extend([(tid, 'HAS_STEP', sid), (sid, 'USES_TOOL', cid),
                                 (cid, 'INSTANCE_OF', 'Tool:' + fixture['tool_name']),
                                 (cid, 'TRIGGERED_BY', fixture['message_id']),
                                 (sid, 'TOUCHED', fixture['entity_id'])])
            if not set(required) <= edge_set:
                raise ContractError(path + ': required fixture edge absent')
        _consistency(document['api'] if path == 'graph' else document['reconnect']['api'],
                     graph, fixture, path)
        canonical = {'nodes': sorted(nodes, key=encode), 'edges': sorted(edges, key=encode)}
        if path == 'graph': result['graph'] = canonical
        else: result['reconnect']['graph'] = canonical
    return result


def _consistency(api, graph, fixture, path):
    """Per-side invariants: a capture must be internally coherent before comparison.

    Every expectation comes from the frozen fixture or from the capture itself,
    never from the other server, so equal-but-corrupt captures fail closed.
    """
    fixture_tasks = {task['task']: task for task in fixture['tasks']}
    by_task = {}
    for trace in api['traces']:
        if trace['task'] in by_task:
            raise ContractError(f'{path}: duplicate task in traces')
        by_task[trace['task']] = trace
    if set(by_task) != set(fixture_tasks):
        raise ContractError(f'{path}: task set differs from fixture')

    api_steps, api_calls = {}, {}
    for name, task in fixture_tasks.items():
        trace = by_task[name]
        if len(trace['steps']) != len(task['steps']):
            raise ContractError(f'{path}: step count differs from fixture')
        for step, spec in zip(trace['steps'], task['steps']):
            if len(step['tool_calls']) != 1:
                raise ContractError(f'{path}: exactly one tool call per step required')
            call = step['tool_calls'][0]
            if call['status'] != spec['status']:
                raise ContractError(f'{path}: tool status differs from fixture')
            if call['duration_ms'] != spec['duration_ms']:
                raise ContractError(f'{path}: tool duration differs from fixture')
            if step['id'] in api_steps or call['id'] in api_calls:
                raise ContractError(f'{path}: duplicate step or call identity')
            api_steps[step['id']] = step
            api_calls[call['id']] = call
        if len(task['steps']) != 3:
            raise ContractError(f'{path}: fixture must declare three calls per task')

    nodes, edges, labels = {}, {}, {}
    for node in graph['nodes']:
        label = node['labels'][0]
        nodes.setdefault(label, {})[node['key']] = node['properties']
        labels[node['key']] = label
    for edge in graph['edges']:
        edges.setdefault(edge['type'], []).append(edge)
        if edge['source'] not in labels or edge['target'] not in labels:
            raise ContractError(f'{path}: edge endpoint is not a node')
    touched_keys = {edge['source'] for group in edges.values() for edge in group}
    touched_keys |= {edge['target'] for group in edges.values() for edge in group}
    if set(labels) - touched_keys:
        raise ContractError(f'{path}: orphan node without any edge')
    seen = set()
    for edge in graph['edges']:
        identity = (edge['source'], edge['type'], edge['target'])
        if identity in seen:
            raise ContractError(f'{path}: duplicate edge')
        seen.add(identity)
    endpoints = {'HAS_STEP': ('ReasoningTrace', 'ReasoningStep'),
                 'USES_TOOL': ('ReasoningStep', 'ToolCall'),
                 'INSTANCE_OF': ('ToolCall', 'Tool'),
                 'INITIATED_BY': ('ReasoningTrace', 'Message'),
                 'TRIGGERED_BY': ('ToolCall', 'Message'),
                 'TOUCHED': ('ReasoningStep', 'Entity'),
                 'HAS_MESSAGE': ('Conversation', 'Message'),
                 'MENTIONS': ('Message', 'Entity')}
    for edge in graph['edges']:
        expected = endpoints[edge['type']]
        if (labels.get(edge['source']), labels.get(edge['target'])) != expected:
            raise ContractError(f'{path}: {edge["type"]} endpoints have unexpected labels')

    def linked(kind, source=None, target=None):
        return [edge for edge in edges.get(kind, ())
                if (source is None or edge['source'] == source)
                and (target is None or edge['target'] == target)]

    graph_traces = nodes.get('ReasoningTrace', {})
    graph_steps = nodes.get('ReasoningStep', {})
    graph_calls = nodes.get('ToolCall', {})
    if set(graph_traces) != {trace['id'] for trace in by_task.values()}:
        raise ContractError(f'{path}: graph traces differ from API traces')
    if set(graph_steps) != set(api_steps) or set(graph_calls) != set(api_calls):
        raise ContractError(f'{path}: graph steps or calls differ from API documents')

    for name, task in fixture_tasks.items():
        trace = by_task[name]
        properties = graph_traces[trace['id']]
        for field in ('session_id', 'task', 'success'):
            if properties.get(field) != trace[field]:
                raise ContractError(f'{path}: trace {field} differs between API and graph')
        if len(linked('INITIATED_BY', trace['id'])) != 1:
            raise ContractError(f'{path}: exactly one INITIATED_BY edge per trace required')
        has_step = linked('HAS_STEP', trace['id'])
        if len(has_step) != len(trace['steps']):
            raise ContractError(f'{path}: HAS_STEP count differs from steps')
        ordered = sorted(has_step, key=lambda edge: edge['properties']['order'])
        if [edge['target'] for edge in ordered] != [step['id'] for step in trace['steps']]:
            raise ContractError(f'{path}: HAS_STEP order differs from API step order')

    for step in api_steps.values():
        if len(linked('HAS_STEP', target=step['id'])) != 1:
            raise ContractError(f'{path}: exactly one HAS_STEP parent per step required')
        if graph_steps[step['id']].get('step_number') != step['step_number']:
            raise ContractError(f'{path}: step_number differs between API and graph')
        orders = {edge['properties']['order'] for edge in linked('HAS_STEP', target=step['id'])}
        if orders != {step['step_number']}:
            raise ContractError(f'{path}: step_number differs from HAS_STEP order')
        uses = linked('USES_TOOL', step['id'])
        if {edge['target'] for edge in uses} != {call['id'] for call in step['tool_calls']}:
            raise ContractError(f'{path}: USES_TOOL targets differ from API tool calls')
        if len(linked('TOUCHED', step['id'])) != 1:
            raise ContractError(f'{path}: exactly one TOUCHED edge per step required')

    for call in api_calls.values():
        properties = graph_calls[call['id']]
        if properties.get('tool_name') != call['tool_name']:
            raise ContractError(f'{path}: tool name differs between API and graph')
        if len(linked('USES_TOOL', target=call['id'])) != 1:
            raise ContractError(f'{path}: exactly one USES_TOOL parent per call required')
        instance = linked('INSTANCE_OF', call['id'])
        if len(instance) != 1:
            raise ContractError(f'{path}: exactly one INSTANCE_OF edge per call required')
        tool = nodes.get('Tool', {}).get(instance[0]['target'], {})
        if tool.get('name') != call['tool_name']:
            raise ContractError(f'{path}: INSTANCE_OF tool differs from the call tool name')
        if len(linked('TRIGGERED_BY', call['id'])) != 1:
            raise ContractError(f'{path}: exactly one TRIGGERED_BY edge per call required')

    stats = api['stats']
    if len(stats) != 1 or stats[0]['name'] != fixture['tool_name']:
        raise ContractError(f'{path}: statistics must name the fixture tool exactly once')
    stats = stats[0]
    tool_key = 'Tool:' + fixture['tool_name']
    if set(nodes.get('Tool', {})) != {tool_key}:
        raise ContractError(f'{path}: exactly one fixture tool node required')
    tool = nodes['Tool'][tool_key]
    total = len(api_calls)
    successful = sum(1 for call in api_calls.values() if call['status'] == 'success')
    graph_failed = sum(1 for call in api_calls.values() if call['status'] in ('error', 'timeout'))
    durations = [call['duration_ms'] for call in api_calls.values()]
    if any(value is None for value in durations):
        raise ContractError(f'{path}: every fixture call carries an explicit duration')
    duration_sum = sum(durations)
    # API failures mean every non-successful call; the persisted graph counter keeps
    # only error/timeout. Each counter is checked against its own documented meaning.
    if stats['total_calls'] != total or tool.get('total_calls') != total:
        raise ContractError(f'{path}: total tool calls differ from recorded calls')
    if stats['successful_calls'] != successful or tool.get('successful_calls') != successful:
        raise ContractError(f'{path}: successful tool calls differ from recorded statuses')
    if stats['failed_calls'] != total - successful:
        raise ContractError(f'{path}: API failed calls must be total minus successes')
    if tool.get('failed_calls') != graph_failed:
        raise ContractError(f'{path}: graph failed counter must count error and timeout')
    if tool.get('total_duration_ms') != duration_sum:
        raise ContractError(f'{path}: graph total duration differs from recorded durations')
    if stats['success_rate'] != successful / total:
        raise ContractError(f'{path}: success rate differs from recorded statuses')
    if stats['avg_duration_ms'] != duration_sum / total:
        raise ContractError(f'{path}: average duration differs from recorded durations')

    if set(nodes.get('Message', {})) != {fixture['message_id']}:
        raise ContractError(f'{path}: exactly one fixture message node required')
    for name in ('created_at', 'last_used_at'):
        if name not in tool:
            raise ContractError(f'{path}: tool {name} absent')
    if moment(tool['created_at']) > moment(tool['last_used_at']):
        raise ContractError(f'{path}: tool created after its last use')
    latest = None
    for name, task in fixture_tasks.items():
        trace = by_task[name]
        properties = graph_traces[trace['id']]
        if 'started_at' not in properties:
            raise ContractError(f'{path}: trace started_at absent')
        started = moment(properties['started_at'])
        completed = (moment(properties['completed_at'])
                     if properties.get('completed_at') is not None else None)
        if completed is not None and completed < started:
            raise ContractError(f'{path}: trace completion precedes its start')
        previous = started
        for step in trace['steps']:
            step_properties = graph_steps[step['id']]
            if 'timestamp' not in step_properties:
                raise ContractError(f'{path}: step timestamp absent')
            stamp = moment(step_properties['timestamp'])
            if stamp < previous:
                raise ContractError(f'{path}: step timestamp precedes its trace or step order')
            if completed is not None and stamp > completed:
                raise ContractError(f'{path}: step timestamp follows trace completion')
            previous = stamp
            for call in step['tool_calls']:
                call_properties = graph_calls[call['id']]
                if 'timestamp' not in call_properties:
                    raise ContractError(f'{path}: tool call timestamp absent')
                recorded = moment(call_properties['timestamp'])
                if recorded < stamp:
                    raise ContractError(f'{path}: tool call precedes its step')
                if completed is not None and recorded > completed:
                    raise ContractError(f'{path}: tool call follows trace completion')
                latest = recorded if latest is None or recorded > latest else latest
    if latest is not None and moment(tool['last_used_at']) < latest:
        raise ContractError(f'{path}: tool last_used_at precedes a recorded call')


def _diff(left, right, path, diffs):
    if isinstance(left, dict) and isinstance(right, dict):
        for key in sorted(left.keys() | right.keys()):
            child = path + '.' + key if path else key
            if key not in left or key not in right:
                diffs.append({'path': child, 'expected_present': key in left, 'actual_present': key in right,
                              'expected': left.get(key), 'actual': right.get(key)})
            else:
                _diff(left[key], right[key], child, diffs)
    elif isinstance(left, list) and isinstance(right, list):
        if len(left) != len(right):
            diffs.append({'path': path + '.length', 'expected': len(left), 'actual': len(right)})
        for index, (one, two) in enumerate(zip(left, right)):
            _diff(one, two, f'{path}[{index}]', diffs)
    elif (type(left) is not type(right) and not (type(left) in (int, float) and type(right) in (int, float))) or left != right:
        diffs.append({'path': path, 'expected': left, 'actual': right})


def compare_gm08(expected, actual, numeric_tolerance=0):
    """Public contract: never throw; retain field values and structural failures."""
    errors, diffs, canonical = [], [], []
    if type(numeric_tolerance) not in (int, float) or numeric_tolerance != 0:
        errors.append('numeric_tolerance: only zero is authorized')
    for side, document in (('expected', expected), ('actual', actual)):
        try:
            canonical.append(canonical_gm08(document))
        except (ValueError, TypeError, KeyError, AttributeError) as error:
            errors.append(f'{side}: {error}')
            canonical.append(deepcopy(document))
    _diff(*canonical, '', diffs)
    errors.extend(diff['path'] + ': different value, type or presence' for diff in diffs)
    items = []
    for name in ('api', 'graph', 'reconnect'):
        item_diffs = [diff for diff in diffs if diff['path'].startswith(name)]
        items.append({'item': name, 'verdict': 'fail' if errors or item_diffs else 'pass', 'diffs': item_diffs})
    return {'comparison': 'fail' if errors or diffs else 'pass', 'numeric_tolerance': 0,
            'errors': errors, 'diffs': diffs, 'items': items}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--expected', required=True, type=Path, help='Raw captured oracle JSON (read only)')
    parser.add_argument('--actual', required=True, type=Path, help='Raw captured product JSON (read only)')
    args = parser.parse_args(argv)
    try:
        verdict = compare_gm08(json.loads(args.expected.read_text()), json.loads(args.actual.read_text()))
    except (OSError, ValueError):
        verdict = {'comparison': 'fail', 'errors': ['capture: unreadable or invalid JSON'], 'diffs': [], 'items': []}
    print(json.dumps(verdict, sort_keys=True, allow_nan=False))
    return 0 if verdict['comparison'] == 'pass' else 2


if __name__ == '__main__':
    raise SystemExit(main())
