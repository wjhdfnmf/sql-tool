"""Writes the extra themes into app/src/app.css and checks every theme for readable contrast.

    python tools/themes.py .            write the themes below into app.css, then check
    python tools/themes.py . --check    only check, change nothing

The check covers the built-in themes too (light, dark, midnight, sepia, high
contrast): text on every background, each syntax colour on the background, on
a selection and on the editor's current line, selections, pending edits,
buttons and warnings. A new theme that fails is not written.

To add a theme: add it to NEW and LABELS here, to THEMES in
crates/core/src/settings.rs, and to the list in app/src/lib/settings.svelte.ts.
"""
import os
import re
import sys

root = sys.argv[1]
check_only = '--check' in sys.argv

ORDER = [
    'bg', 'panel', 'panel-2', 'border', 'text', 'muted', 'zebra', 'placeholder', 'accent', 'accent-text',
    'selection', 'changed', 'deleted', 'danger', 'ok', 'warn-bg', 'warn-border', 'warn-text',
    'syn-keyword', 'syn-string', 'syn-number', 'syn-comment', 'syn-name',
]


def theme(scheme, *values):
    assert len(values) == len(ORDER), len(values)
    return scheme, dict(zip(ORDER, values))


# name: (light or dark, colours in ORDER)
NEW = {
    # ---- soft, light ----
    'mist': theme('light', '#f5f7fa', '#ebeff4', '#dfe5ec', '#c5cfda', '#232d38', '#566270', '#eff3f7', '#d9e1ea',
                  '#2f62a0', '#ffffff', '#d3e2f4', '#fbeeb5', '#f6d9d6', '#a92f25', '#256f45',
                  '#f8ecc4', '#cfa846', '#4f3a00', '#6f3fa8', '#954016', '#1c6650', '#5c6675', '#245a9c'),
    'rose': theme('light', '#fff7f9', '#fbecf0', '#f5dde4', '#e3bcc9', '#35212b', '#75525f', '#fdf1f4', '#f0d6de',
                  '#a8305f', '#ffffff', '#f8d5e1', '#fbeab0', '#f6d0cb', '#a5252f', '#34703b',
                  '#f9ecc2', '#cfa846', '#4f3a00', '#96266b', '#8f4116', '#226659', '#80636e', '#35569c'),
    'lavender': theme('light', '#f9f7fd', '#efebf8', '#e3dcf1', '#cabfe0', '#272138', '#5f5778', '#f4f1fa', '#ddd5ec',
                      '#5c40b0', '#ffffff', '#ddd4f5', '#fbeeb5', '#f6d6d6', '#a82b33', '#2c6f48',
                      '#f8ecc4', '#cfa846', '#4f3a00', '#7032b8', '#96401a', '#1a6553', '#635c79', '#2a56a2'),
    'sage': theme('light', '#f6f9f4', '#eaf0e6', '#dde6d8', '#bfcdb8', '#1f2a1a', '#55634f', '#f0f5ed', '#d6e0d0',
                  '#32702d', '#ffffff', '#d3e6cd', '#f9ecb0', '#f4d6d0', '#a52e25', '#2a6b2c',
                  '#f6ebc0', '#cfa846', '#4f3a00', '#72368f', '#8f4116', '#1a6262', '#5c6856', '#24578f'),
    # ---- soft, dark ----
    'graphite': theme('dark', '#1b1e22', '#23272c', '#2d3238', '#444c55', '#e2e6ea', '#9ba5b0', '#20242a', '#333a42',
                      '#3fb6a8', '#06201d', '#1f4744', '#3c3518', '#47231f', '#f08a80', '#6fd08c',
                      '#3a3115', '#7a6420', '#f0dca0', '#7fc8f8', '#e9b872', '#9be29b', '#8e98a4', '#5fd3c4'),
    'forest': theme('dark', '#111a14', '#17231b', '#1f2f24', '#35503d', '#dfeadf', '#96ab98', '#142018', '#27392d',
                    '#7cc66a', '#0c1a0a', '#22492c', '#3d3a16', '#4a2420', '#f28b7d', '#86d98a',
                    '#383214', '#7a6420', '#f0dca0', '#e6c45a', '#f0a875', '#8fd6c8', '#869a86', '#9ad1f5'),
    'orchid': theme('dark', '#1d1422', '#27192e', '#33213c', '#553a62', '#f1e6f5', '#b6a0c0', '#221728', '#3b2846',
                    '#f17ac0', '#2a0a1e', '#5a2a64', '#3b300c', '#55182a', '#ff8696', '#88dba0',
                    '#3a3012', '#7a6420', '#f0dca0', '#f59ad0', '#f5c97a', '#8fe0c8', '#a691b1', '#9ec2ff'),
    # ---- bold, dark ----
    'ember': theme('dark', '#1c1412', '#261a17', '#33231e', '#573a31', '#f3e6df', '#bda297', '#211715', '#3b2923',
                   '#ff7a3d', '#1f0d04', '#5e2e19', '#3a2d0d', '#5a1f1c', '#ff8a80', '#8fd88a',
                   '#3d3010', '#7a6420', '#f0dca0', '#ff8f5a', '#ffd27a', '#9be0b0', '#a89288', '#8cc8ff'),
    'neon': theme('dark', '#0e0b1a', '#151127', '#1f1938', '#3f3778', '#f1eeff', '#aaa4d0', '#120f22', '#282148',
                  '#ff4fd8', '#1a0316', '#3f2278', '#33290b', '#55162c', '#ff7a90', '#5dfcb0',
                  '#352c10', '#7a6420', '#f0dca0', '#ff6fe0', '#ffe066', '#5dfcb0', '#a19ac9', '#5fd8ff'),
    'cobalt': theme('dark', '#0f2540', '#14304f', '#1b3d63', '#34629a', '#eaf2ff', '#a9c0db', '#122a48', '#244870',
                    '#ffc600', '#1a1400', '#194b88', '#4a4012', '#5a2030', '#ff9a9a', '#7fe59a',
                    '#3f3610', '#8a7020', '#f5e3a8', '#ff9d00', '#a5ff90', '#ff9ae0', '#97b6d8', '#9effff'),
}

LABELS = {
    'mist': ('Mist', 'Soft cool greys and blue'),
    'rose': ('Rose', 'Soft pinks'),
    'lavender': ('Lavender', 'Soft purples'),
    'sage': ('Sage', 'Soft greens'),
    'graphite': ('Graphite', 'Charcoal and steel with teal'),
    'forest': ('Forest', 'Deep greens'),
    'orchid': ('Orchid', 'Plum with pink'),
    'ember': ('Ember', 'Bold: near-black with orange'),
    'neon': ('Neon', 'Bold: violet-black with vivid pink and cyan'),
    'cobalt': ('Cobalt', 'Bold: deep blue with yellow'),
}


def luminance(color):
    r, g, b = (int(color[i:i + 2], 16) / 255 for i in (1, 3, 5))

    def channel(c):
        return c / 12.92 if c <= 0.03928 else ((c + 0.055) / 1.055) ** 2.4
    return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)


def contrast(a, b):
    la, lb = sorted((luminance(a), luminance(b)), reverse=True)
    return (la + 0.05) / (lb + 0.05)


def mix(a, b, share):
    """`share` of colour a over colour b, as the editor's active line and match highlight do."""
    parts = [round(int(a[i:i + 2], 16) * share + int(b[i:i + 2], 16) * (1 - share)) for i in (1, 3, 5)]
    return '#' + ''.join(f'{p:02x}' for p in parts)


# (foreground, background, least acceptable ratio, what it is)
CHECKS = [
    ('text', 'bg', 9.0, 'text'),
    ('text', 'panel', 8.0, 'text on panels'),
    ('text', 'panel-2', 7.0, 'text on hovered rows'),
    ('text', 'zebra', 8.5, 'text on striped rows'),
    ('text', 'selection', 7.0, 'text on a selection'),
    ('muted', 'bg', 4.5, 'secondary text'),
    ('muted', 'panel', 4.5, 'secondary text on panels'),
    ('syn-keyword', 'bg', 4.5, 'keywords'),
    ('syn-string', 'bg', 4.5, 'strings'),
    ('syn-number', 'bg', 4.5, 'numbers'),
    ('syn-comment', 'bg', 4.5, 'comments'),
    ('syn-name', 'bg', 4.5, 'names'),
    ('accent-text', 'accent', 4.5, 'text on the main button'),
    ('accent', 'bg', 3.0, 'accent lines and marks'),
    ('accent', 'changed', 4.5, 'a pending edit in the grid'),
    ('danger', 'bg', 4.5, 'errors'),
    ('ok', 'bg', 4.0, 'success marks'),
    ('warn-text', 'warn-bg', 7.0, 'warnings'),
    ('border', 'bg', 1.25, 'borders'),
]
# Syntax colours also have to read on a selection and on the editor's current line.
SYNTAX = ['syn-keyword', 'syn-string', 'syn-number', 'syn-comment', 'syn-name']


def problems(name, colours):
    found = []
    for fg, bg, least, what in CHECKS:
        ratio = contrast(colours[fg], colours[bg])
        if ratio < least:
            found.append(f'{name}: {what} ({fg} on {bg}) is {ratio:.2f}, needs {least}')
    active_line = mix(colours['selection'], colours['bg'], 0.45)
    for syn in SYNTAX:
        for background, label, least in ((colours['selection'], 'a selection', 3.0), (active_line, 'the current line', 4.5)):
            ratio = contrast(colours[syn], background)
            if ratio < least:
                found.append(f'{name}: {syn} on {label} is {ratio:.2f}, needs {least}')
    if contrast(colours['selection'], colours['bg']) < 1.12:
        found.append(f'{name}: a selection hardly differs from the background')
    return found


def existing(css):
    """The themes already in app.css, as name -> colours."""
    out = {}
    base = re.search(r':root \{(.*?)\n\}', css, re.S).group(1)
    light = dict(re.findall(r'--([a-z0-9-]+): (#[0-9a-fA-F]{6});', base))
    out['light'] = light
    for name, body in re.findall(r":root\[data-theme='([a-z]+)'\] \{(.*?)\n\}", css, re.S):
        colours = dict(light)
        colours.update(dict(re.findall(r'--([a-z0-9-]+): (#[0-9a-fA-F]{6});', body)))
        out[name] = colours
    return out


path = os.path.join(root, 'app/src/app.css')
css = open(path, encoding='utf-8').read().replace('\r\n', '\n')

MARK = '/* ---- More themes (written by the theme script; edit the colours there) ---- */'
ANCHOR = "* {\n  box-sizing: border-box;"
if MARK in css:
    # Written before: take the old blocks out, keeping the general rules that follow them.
    css = css[:css.index(MARK)].rstrip('\n') + '\n\n' + css[css.index(ANCHOR):]

blocks = [MARK]
for name, (scheme, colours) in NEW.items():
    lines = [f":root[data-theme='{name}'] {{", f'  color-scheme: {scheme};']
    lines += [f'  --{key}: {colours[key]};' for key in ORDER]
    lines.append('}')
    blocks.append('\n'.join(lines))

# The themes go right after the last built-in theme block, before the general rules.
anchor = css.index(ANCHOR)
new_css = css[:anchor].rstrip('\n') + '\n\n' + '\n\n'.join(blocks) + '\n\n' + css[anchor:]

every = existing(new_css)
issues = []
for name, colours in every.items():
    issues += problems(name, colours)
print(f'{len(every)} themes checked')
for line in issues:
    print('  ' + line)
if not issues:
    print('  every colour pair is within its limit')

if not check_only and not [i for i in issues if i.split(':')[0] in NEW]:
    open(path, 'w', encoding='utf-8', newline='\n').write(new_css)
    print('app.css written')
elif not check_only:
    print('app.css NOT written: fix the new themes above first')
