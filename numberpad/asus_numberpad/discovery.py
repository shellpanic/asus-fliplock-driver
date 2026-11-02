"""Read-only discovery. Never retain event or I2C bus numbers in configuration."""
from pathlib import Path
import re

MODEL = 'ASUS EXPERTBOOK B5402FVA_B5402FVA'


def read(path):
    return path.read_text().strip()


def probe(sys=Path('/sys'), dev=Path('/dev'), require_switch=True):
    model = read(sys / 'class/dmi/id/product_name')
    matches, switches = [], []
    for event in sorted((sys / 'class/input').glob('event*')):
        device = event / 'device'
        try:
            name = read(device / 'name')
            vendor = read(device / 'id/vendor').upper()
            product = read(device / 'id/product').upper()
            ev = int(read(device / 'capabilities/ev').replace(' ', ''), 16)
            sw = int(read(device / 'capabilities/sw').replace(' ', ''), 16)
        except (OSError, ValueError):
            continue
        if sw & 2 and not ev & 2:  # SW_TABLET_MODE, no physical keyboard access
            switches.append({'event': str(dev / event.name), 'sysfs': str(event.resolve())})
        if (model != MODEL or vendor != '04F3' or product != '3230'
                or not name.startswith('ASUE') or not name.endswith('Touchpad')):
            continue
        physical = device.resolve()
        adapters = [p for p in physical.parents if re.fullmatch(r'i2c-\d+', p.name)]
        if len(adapters) != 1:
            continue
        adapter = adapters[0]
        matches.append({'event': str(dev / event.name), 'sysfs': str(event.resolve()),
                        'name': name, 'adapter': str(dev / adapter.name),
                        'adapter_sysfs': str((sys / 'class/i2c-dev' / adapter.name).resolve()),
                        'vendor': vendor, 'product': product, 'profile': 'b7402'})
    if len(matches) != 1:
        raise ValueError(f'Refusing hardware: expected one exact {MODEL} / 04F3:3230 touchpad; found {len(matches)}')
    if require_switch and len(switches) != 1:
        raise ValueError(f'Refusing hardware: expected one tablet-mode switch without keyboard keys; found {len(switches)}')
    return {'model': model, **matches[0], 'switch': switches[0] if len(switches) == 1 else None,
            'uinput': str(dev / 'uinput'), 'physical_acceptance': 'pending'}
