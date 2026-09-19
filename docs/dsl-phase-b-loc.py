"""Count Phase B line changes, separating reviewed test-only source modules.

Usage: python3 docs/dsl-phase-b-loc.py BASE SOURCE
Counts blank/comment lines and uses Git without rename detection. Inline cfg(test) ranges
and external test modules are test Rust; docs are Markdown plus canonical JSON fixtures.
Manifests, lockfiles, UI diagnostics, Nix and evidence scripts are reported separately.
"""

import collections, json, re, subprocess, sys
BASE, SOURCE = sys.argv[1:]
def git(*args):
    return subprocess.check_output(['git', *args], text=True)
def masks(rev, path):
    result = subprocess.run(['git', 'show', rev + ':' + path], capture_output=True, text=True)
    lines = result.stdout.splitlines()
    marked = set()
    for start, line in enumerate(lines):
        if line.strip() != '#[cfg(test)]':
            continue
        end = start + 1
        while lines[end].strip().startswith('#['):
            end += 1
        if re.fullmatch(r'mod \w+ \{', lines[end]):
            end = next(i for i in range(end + 1, len(lines)) if lines[i] == '}')
        else:
            while not lines[end].rstrip().endswith(';'):
                end += 1
        marked.update(range(start + 1, end + 2))
    return marked
counts = collections.defaultdict(lambda: [0, 0])
files = git('diff', '--name-only', '--no-renames', BASE, SOURCE).splitlines()
for path in files:
    if path.endswith('.rs'):
        category = 'test Rust' if '/tests/' in path or path.endswith(('tests.rs', '_test.rs', '_extremes.rs')) else 'production Rust'
    elif path.endswith('.md') or path.startswith('docs/contracts/'):
        category = 'documentation'
    else:
        category = 'manifest/lock/UI/other'
    before = masks(BASE, path) if category == 'production Rust' else set()
    after = masks(SOURCE, path) if category == 'production Rust' else set()
    patch = git('diff', '--no-renames', '--unified=0', BASE, SOURCE, '--', path)
    for line in patch.splitlines():
        if line.startswith('@@ '):
            match = re.match(r'@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@', line)
            old, new = map(int, match.groups())
        elif line.startswith(('---', '+++')):
            continue
        elif line.startswith('-'):
            counts['test Rust' if old in before else category][1] += 1
            old += 1
        elif line.startswith('+'):
            counts['test Rust' if new in after else category][0] += 1
            new += 1
print(json.dumps({'baseline': BASE, 'source': SOURCE, 'categories': {k: {'added': a, 'removed': d, 'net': a-d} for k, (a, d) in counts.items()}}, indent=2))
