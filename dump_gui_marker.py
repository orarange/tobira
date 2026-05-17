import sys
with open('src/gui.rs', 'r', encoding='utf-8') as f:
    content = f.read()

for line in content.splitlines():
    if line.startswith('>>>>>>>'):
        print(line)
        break
