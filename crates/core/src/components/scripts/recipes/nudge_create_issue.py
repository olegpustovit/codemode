# Create an issue by team key and ordinary workflow status name.
required = ['teamKey', 'statusName', 'title']
if any([not isinstance(args.get(key), str) or not args[key].strip() for key in required]):
    raise ToolError('args requires non-empty teamKey, statusName and title strings')
if any([key not in ['teamKey', 'statusName', 'title', 'description', 'priority'] for key in args]):
    raise ToolError('supported args: teamKey, statusName, title, description, priority')
if len(args['title']) > 500:
    raise ToolError('title must be at most 500 characters')
if args.get('description') is not None and not isinstance(args['description'], str):
    raise ToolError('description must be a string')
if args.get('priority') is not None and (type(args['priority']) != int or args['priority'] < 0 or args['priority'] > 4):
    raise ToolError('priority must be an integer from 0 to 4')
teams = await tools.$SOURCE.list_teams.all()
teams = [team for team in teams if team['key'].lower() == args['teamKey'].strip().lower()]
if len(teams) != 1:
    raise ToolError('teamKey must match exactly one connected team')
statuses = await tools.$SOURCE.list_issue_statuses.all(teamId=teams[0]['id'])
statuses = [status for status in statuses if status['name'].lower() == args['statusName'].strip().lower() and status.get('category') != 'duplicate']
if len(statuses) != 1:
    raise ToolError('statusName must match exactly one ordinary workflow status in the team')
payload = {'teamId': teams[0]['id'], 'statusId': statuses[0]['id'], 'title': args['title']}
for key in ['description', 'priority']:
    if args.get(key) is not None:
        payload[key] = args[key]
created = await tools.$SOURCE.create_issue(**payload)
if not isinstance(created, dict) or not isinstance(created.get('data'), dict) or not created['data'].get('id'):
    raise ToolError('create_issue returned an unexpected result; verify the issue before retrying')
return {key: created['data'][key] for key in ['id', 'identifier', 'title', 'statusId'] if key in created['data']}
