import asyncio
import json
ToolError = RuntimeError

def pick(rows, **match):
    if len(match) != 1:
        raise TypeError('pick requires one field, for example pick(rows, name="Backlog")')
    field = list(match)[0]
    wanted = match[field]
    choices = []
    more = False
    for row in rows:
        if not isinstance(row, dict) or field not in row:
            continue
        value = row[field]
        if value == wanted:
            return row
        if not isinstance(value, (str, int, float, bool)) and value is not None:
            continue
        label = repr(value[:40] if isinstance(value, str) else value)
        if len(label) > 48:
            label = label[:48] + '…'
        if label not in choices:
            if len(choices) < 6:
                choices.append(label)
            else:
                more = True
    if more:
        choices.append('…')
    raise LookupError(f'No match for {field[:40]}. Available values: ' + (', '.join(choices) or '(none)'))


_print = print

def _sample_output(value, depth=0):
    # shortcut: ordinary stdout and nested containers up to six levels; keep unusually deep diagnostics intact.
    if depth == 6:
        return value
    if isinstance(value, list):
        if len(value) >= 20:
            for row in value:
                if not isinstance(row, dict):
                    break
            else:
                fields = ', '.join([f'{str(key)[:32]}: {type(value[0][key]).__name__}' for key in list(value[0])[:6]])
                return [_sample_output(row, depth + 1) for row in value[:3]] + [
                    f'… {len(value) - 3} more rows hidden. Count, filter or group these in code and print only the result. Row shape: {{{fields}}}'
                ]
        children = enumerate(value)
    elif isinstance(value, dict):
        children = value.items()
    else:
        return value
    preview = value
    for key, item in children:
        sampled = _sample_output(item, depth + 1)
        if sampled is not item:
            if preview is value:
                preview = value.copy()
            preview[key] = sampled
    return preview


def _printed_value(value):
    # shortcut: parse JSON container strings up to 256 KiB; use structured values for bigger dumps or Python repr strings.
    if isinstance(value, str) and 40 <= len(value) <= 262144 and value.lstrip().startswith(('[', '{')):
        try:
            parsed = json.loads(value)
        except ValueError:
            return value
        sampled = _sample_output(parsed)
        return value if sampled is parsed else json.dumps(sampled)
    return _sample_output(value)


def print(*values, **kwargs):
    _print(*[_printed_value(value) for value in values], **kwargs)


class _Tool:
    def __init__(self, path, aliases, full, pagination):
        self.path = path
        self.aliases = aliases
        self.full = full
        self.pagination = pagination
        if pagination is not None:
            self.all = self._all

    async def __call__(self, _fields=None, **kwargs):
        _check_fields(_fields)
        args = {self.aliases.get(k, k): v for k, v in kwargs.items()}
        return _project(json.loads(await _call(self.path, json.dumps(args))), _fields)

    async def _all(self, max_pages=20, _fields=None, **kwargs):
        if self.pagination is None:
            raise ToolError('tool does not have cursor pagination')
        if type(max_pages) != int or max_pages < 1 or max_pages > 100:
            raise ToolError('max_pages must be an integer from 1 to 100')
        _check_fields(_fields)
        rows = []
        seen = []
        if kwargs.get('cursor') is not None:
            seen.append([kwargs['cursor']] + [kwargs.get(pair[0]) for pair in self.pagination['extras']])
        for _ in range(max_pages):
            page = await self(**kwargs)
            items = page
            for key in self.pagination['items']:
                if not isinstance(items, dict) or key not in items:
                    raise ToolError('pagination response is missing its items path')
                items = items[key]
            container = page
            for key in self.pagination['cursor'][:-1]:
                if not isinstance(container, dict) or key not in container:
                    raise ToolError('pagination response is missing its cursor path')
                container = container[key]
            cursor_key = self.pagination['cursor'][-1]
            if not isinstance(container, dict):
                raise ToolError('pagination cursor container must be an object')
            if cursor_key not in container and not self.pagination['optional']:
                raise ToolError('pagination response is missing its cursor path')
            cursor = container.get(cursor_key)
            if items is None:
                items = []
            if not isinstance(items, list):
                raise ToolError('pagination items must be a list')
            rows.extend(_project(items, _fields))
            if cursor is None or cursor == '':
                return rows
            if type(cursor) not in (str, int):
                raise ToolError('pagination cursor must be a string, integer or None')
            state = [cursor] + [container.get(pair[1]) for pair in self.pagination['extras']]
            if state in seen:
                raise ToolError('pagination cursor repeated; stopped to avoid a loop')
            seen.append(state)
            kwargs['cursor'] = cursor
            for pair in self.pagination['extras']:
                if pair[1] in container:
                    kwargs[pair[0]] = container[pair[1]]
                else:
                    kwargs.pop(pair[0], None)
        raise ToolError('pagination max_pages reached; narrow filters or increase max_pages')

def help(tool):
    return tool.full


def _check_fields(fields):
    if fields is not None and (not isinstance(fields, list) or any([not isinstance(k, str) for k in fields])):
        raise ToolError('_fields must be a list of field names')

def _project(value, fields):
    if fields is None:
        return value
    if isinstance(value, list):
        return [{k: row[k] for k in fields if k in row} if isinstance(row, dict) else row for row in value]
    if isinstance(value, dict):
        # Preserve list envelopes and pagination keys while trimming their rows.
        if isinstance(value.get('data'), (dict, list)):
            return {k: _project(v, fields) if k == 'data' else v for k, v in value.items()}
        lists = [k for k in ('items', 'results', 'entries', 'rows', 'issues') if isinstance(value.get(k), list)]
        if lists:
            return {k: _project(v, fields) if k in lists else v for k, v in value.items()}
        return {k: value[k] for k in fields if k in value}
    return value

async def _search(query):
    return json.loads(await _call('@codemode/search', json.dumps({'query': query})))

class _Memo:
    def get(self, key, default=None):
        result = json.loads(_call_sync('@codemode/memo', json.dumps({'op': 'get', 'key': key})))
        return result['value'] if result['found'] else default

    def set(self, key, value):
        return json.loads(_call_sync('@codemode/memo', json.dumps({'op': 'set', 'key': key, 'value': value})))

memo = _Memo()

class _Administration:
    def list_workspaces(self):
        return json.loads(_call_sync('@codemode/admin/list_workspaces', json.dumps({})))

    def save_workspace(self, name, sourceIds, id=None):
        return json.loads(_call_sync('@codemode/admin/save_workspace', json.dumps({'name': name, 'sourceIds': sourceIds, 'id': id})))

    def delete_workspace(self, id):
        return json.loads(_call_sync('@codemode/admin/delete_workspace', json.dumps({'id': id})))

    def list_integrations(self):
        return json.loads(_call_sync('@codemode/admin/list_integrations', json.dumps({})))

    def list_sources(self):
        return json.loads(_call_sync('@codemode/admin/list_sources', json.dumps({})))

    def save_source(self, source, id=None):
        return json.loads(_call_sync('@codemode/admin/save_source', json.dumps({'source': source, 'id': id})))

    def delete_source(self, id):
        return json.loads(_call_sync('@codemode/admin/delete_source', json.dumps({'id': id})))

    def sync_source(self, id):
        return json.loads(_call_sync('@codemode/admin/sync_source', json.dumps({'id': id})))

    def list_accounts(self):
        return json.loads(_call_sync('@codemode/admin/list_accounts', json.dumps({})))

    def source_connection_url(self, id):
        return json.loads(_call_sync('@codemode/admin/source_connection_url', json.dumps({'id': id})))

class _Codemode:
    admin = _Administration()

    def save_script(self, name, description, code, scope="workspace"):
        """Save reusable Python for this workspace and return its scriptId."""
        return json.loads(_call_sync('@codemode/save_script', json.dumps({
            'name': name, 'description': description, 'code': code, 'scope': scope
        })))

codemode = _Codemode()
