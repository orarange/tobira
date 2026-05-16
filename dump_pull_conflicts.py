import sys

with open('pull_conflicts.md', 'w', encoding='utf-8') as out:
    for file in ['HANDOFF.md', 'src/css.rs', 'src/layout.rs']:
        with open(file, 'r', encoding='utf-8') as f:
            content = f.read()
        
        in_conflict = False
        conflict_lines = []
        conflicts = []
        
        for line in content.splitlines(True):
            if line.startswith('<<<<<<< HEAD'):
                in_conflict = True
                conflict_lines = [line]
            elif line.startswith('>>>>>>>'):
                if in_conflict:
                    conflict_lines.append(line)
                    conflicts.append("".join(conflict_lines))
                    in_conflict = False
            elif in_conflict:
                conflict_lines.append(line)
                
        out.write(f"--- {file} ---\n")
        for i, c in enumerate(conflicts):
            out.write(f"Conflict {i+1}:\n{c}\n")
