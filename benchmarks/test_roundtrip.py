"""Round-trip test for multiple YXDB files."""
import sigilyx as yx
import os
import sys

test_files = [
    'sigilyx/test_files/AllTypes.yxdb',
    'sigilyx/test_files/NullValues.yxdb', 
    'sigilyx/test_files/ManyRecords.yxdb',
    'sigilyx/test_files/Strings.yxdb',
    'sigilyx/test_files/People.yxdb',
    'sigilyx/test_files/SingleColumn.yxdb',
]

all_passed = True
for path in test_files:
    name = os.path.basename(path)
    print(f'Testing {name}...')
    df1 = yx.read_yxdb(path)
    tmp = path.replace('.yxdb', '_roundtrip.yxdb')
    yx.write_yxdb(tmp, df1)
    df2 = yx.read_yxdb(tmp)
    rows = df1.height
    cols = df1.width
    match = df1.equals(df2)
    print(f'  Rows: {rows}, Cols: {cols}, Match: {match}')
    os.unlink(tmp)
    if not match:
        print('  FAILED!')
        all_passed = False

if all_passed:
    print('\n✅ All round-trip tests PASSED!')
else:
    print('\n❌ Some tests FAILED!')
    sys.exit(1)
