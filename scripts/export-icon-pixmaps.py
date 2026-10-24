#!/usr/bin/env python3
"""Export network-order ARGB for SNI IconPixmap. Build-time only: PySide6."""
from pathlib import Path
from PySide6.QtGui import QImage

root = Path(__file__).resolve().parents[1]
dest = root / 'assets/icons/argb'
dest.mkdir(parents=True, exist_ok=True)
for name in ('devtray', 'devtray-starting', 'devtray-error', 'devtray-idle'):
    for size in (16, 22, 24, 32, 48):
        image = QImage(str(root / f'assets/icons/hicolor/{size}x{size}/apps/{name}.png'))
        if image.isNull() or image.width() != size or image.height() != size:
            raise RuntimeError(f'Invalid icon: {name} {size}')
        # SNI uses bytes A,R,G,B, independent of host endianness; not PNG data.
        data = bytearray()
        for y in range(size):
            for x in range(size):
                c = image.pixelColor(x, y)
                data.extend((c.alpha(), c.red(), c.green(), c.blue()))
        (dest / f'{name}-{size}.argb').write_bytes(data)
