# Exact monthly posting totals for one account and its descendants, separated by currency.
for key in ['from', 'to', 'account']:
    if not isinstance(args.get(key), str) or not args[key].strip():
        raise ToolError('args requires non-empty from, to and account strings')
if any([key not in ['from', 'to', 'account', 'status', 'tag', 'max_pages'] for key in args]):
    raise ToolError('supported args: from, to, account, status, tag, max_pages')
filters = {'from_': args['from'], 'to': args['to'], 'account': args['account'], 'limit': 100}
for key in ['status', 'tag']:
    if args.get(key) is not None:
        filters[key] = args[key]
rows = await tools.$SOURCE.entries_list.all(max_pages=args.get('max_pages', 20), _fields=['date', 'postings'], **filters)
totals = {}
for row in rows:
    month = row['date'][:7]
    if len(month) != 7 or month[4] != '-' or not month[:4].isdigit() or not month[5:].isdigit() or int(month[5:]) < 1 or int(month[5:]) > 12:
        raise ToolError('entry date must have a valid YYYY-MM month')
    for posting in row.get('postings') or []:
        account = posting['account']
        if account != args['account'] and not account.startswith(args['account'] + ':'):
            continue
        amount = posting['amount']
        value = amount['v']
        currency = amount['c']
        if not isinstance(value, str) or not isinstance(currency, str) or not currency:
            raise ToolError('posting amount requires decimal string v and currency c')
        negative = value.startswith('-')
        parts = (value[1:] if negative else value).split('.')
        if len(parts) > 2 or any([not part.isdigit() for part in parts]):
            raise ToolError('posting amount is not an exact decimal')
        scale = len(parts[1]) if len(parts) == 2 else 0
        number = int(''.join(parts)) * (-1 if negative else 1)
        if month not in totals:
            totals[month] = {}
        previous = totals[month].get(currency, [0, 0])
        common = max(previous[1], scale)
        totals[month][currency] = [previous[0] * 10 ** (common - previous[1]) + number * 10 ** (common - scale), common]
months = {}
for month in sorted(totals):
    months[month] = {}
    for currency in sorted(totals[month]):
        number, scale = totals[month][currency]
        digits = str(abs(number))
        if scale:
            digits = '0' * max(0, scale + 1 - len(digits)) + digits
            digits = digits[:-scale] + '.' + digits[-scale:]
        months[month][currency] = ('-' if number < 0 else '') + digits
return {'account': args['account'], 'months': months}
