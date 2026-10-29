"""Project-scoped resource choices, independent of the visible table and agent state."""
from pathlib import Path


def cleanable(usage, artifact):
    return bool(usage.get('complete') and usage.get('cleaning_allowed') and artifact.get('deletable'))


class ResourceSelection:
    def __init__(self):
        self.entries = {}

    def set_checked(self, usage, artifact, checked):
        key = (usage['project'], artifact['path'])
        if not checked:
            self.entries.pop(key, None)
            return
        relative = Path(artifact['path'])
        root = Path(usage['root'])
        if not cleanable(usage, artifact) or not root.is_absolute() or relative.is_absolute() or '..' in relative.parts or relative == Path('.'):
            return
        self.entries[key] = dict(artifact, project=usage['project'], root=str(root), absolute_path=str(root / relative))

    def reconcile(self, usage):
        """Drop choices invalidated by a new root, missing directory, or failed evidence."""
        artifacts = {a['path']: a for a in usage.get('artifacts', [])}
        dropped = 0
        for key, entry in list(self.entries.items()):
            if key[0] != usage['project']:
                continue
            artifact = artifacts.get(key[1])
            if entry['root'] != usage['root'] or artifact is None or not cleanable(usage, artifact):
                self.entries.pop(key)
                dropped += 1
            else:
                self.set_checked(usage, artifact, True)
        return dropped

    def remove(self, key):
        self.entries.pop(tuple(key), None)

    def clear(self):
        self.entries.clear()

    def contains(self, project, path):
        return (project, path) in self.entries

    def groups(self):
        return {name: [dict(entry) for key, entry in self.entries.items() if key[0] == name]
                for name in sorted({key[0] for key in self.entries})}

    @property
    def allocated(self):
        return sum(entry['allocated'] for entry in self.entries.values())


def human_bytes(n):
    value = float(n or 0)
    for unit in ('B', 'KiB', 'MiB', 'GiB', 'TiB'):
        if value < 1024 or unit == 'TiB':
            return f'{int(value)} B' if unit == 'B' else f'{value:.1f} {unit}'
        value /= 1024.0
