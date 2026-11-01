"""Generate our own geometric test font. Requires fontTools; no system/user font input.
Units/em 1000; .notdef and A are 500x700 boxes; space has advance 300.
B is a triangle. Glyphs advance 600, hhea ascent 800, descent -200.
This synthetic fixture and generator are AGPL-3.0-or-later like pm-recorder.
"""
from pathlib import Path
from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen
fb = FontBuilder(1000, isTTF=True)
order = ['.notdef', 'space', 'A', 'B']
fb.setupGlyphOrder(order)
fb.setupCharacterMap({32: 'space', 65: 'A', 66: 'B', 0x5D0: 'A', 0x5D1: 'B'})
glyphs = {}
for name in order:
    pen = TTGlyphPen(None)
    if name != 'space':
        pen.moveTo((0, 0))
        pen.lineTo((500, 0))
        pen.lineTo((500 if name != 'B' else 0, 700))
        if name != 'B':
            pen.lineTo((0, 700))
        pen.closePath()
    glyphs[name] = pen.glyph()
fb.setupGlyf(glyphs)
fb.setupHorizontalMetrics({g: (300 if g == 'space' else 600, 0) for g in order})
fb.setupHorizontalHeader(ascent=800, descent=-200)
fb.setupOS2(sTypoAscender=800, sTypoDescender=-200, usWinAscent=800, usWinDescent=200)
fb.setupNameTable({'familyName': 'Recorder Test', 'styleName': 'Regular',
                  'uniqueFontIdentifier': 'pm-recorder-test-1',
                  'fullName': 'Recorder Test', 'psName': 'RecorderTest'})
fb.setupPost()
fb.setupMaxp()
fb.font.recalcTimestamp = False
fb.font['head'].created = fb.font['head'].modified = 2082844800  # 1970-01-01
fb.save(Path(__file__).with_name('recorder-test.ttf'))

# A second font separates the hhea box from the em box, with explicit BASE
# hanging/ideographic/roman coordinates. No external typeface is copied.
from fontTools.ttLib import newTable
from fontTools.ttLib.tables import otTables
fb.font['hhea'].ascent, fb.font['hhea'].descent = 1200, -300
fb.save(Path(__file__).with_name('recorder-metrics.ttf'))
base = newTable('BASE')
base.table = otTables.BASE()
base.table.Version = 0x00010000
axis = otTables.Axis()
axis.BaseTagList = otTables.BaseTagList()
axis.BaseTagList.BaselineTag = ['hang', 'ideo', 'idtp', 'romn']
axis.BaseScriptList = otTables.BaseScriptList()
record = otTables.BaseScriptRecord()
record.BaseScriptTag = 'latn'
record.BaseScript = otTables.BaseScript()
record.BaseScript.DefaultMinMax = None
record.BaseScript.BaseLangSysRecord = []
values = otTables.BaseValues()
values.DefaultIndex = 3
values.BaseCoord = []
for value in (600, -250, 750, 0):
    coord = otTables.BaseCoord()
    coord.Format = 1
    coord.Coordinate = value
    values.BaseCoord.append(coord)
record.BaseScript.BaseValues = values
axis.BaseScriptList.BaseScriptRecord = [record]
base.table.HorizAxis, base.table.VertAxis = axis, None
fb.font['BASE'] = base
fb.save(Path(__file__).with_name('recorder-baselines.ttf'))
