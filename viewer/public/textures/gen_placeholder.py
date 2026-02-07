#!/usr/bin/env python3
from PIL import Image
img = Image.new('RGB', (2048, 1024), (34, 85, 170))
img.save('/home/sksat/prog/orts/viewer/public/textures/earth_blue_marble.jpg')
print('Placeholder image created successfully')
