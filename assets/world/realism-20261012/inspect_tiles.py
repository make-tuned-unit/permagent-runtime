#!/usr/bin/env python3
"""Read-only image QA: measured edges and lossless repeat/offset previews.

Uses ffmpeg solely to decode PNG pixels, Python stdlib for statistics and PNG
preview encoding. Originals are never changed. No paid or network operations.
"""
from pathlib import Path
import hashlib
import json
import math
import statistics
import struct
import subprocess
import sys
import zlib

BASE = Path(__file__).resolve().parent


def png(path, w, h, rgb):
    def chunk(kind, value):
        return struct.pack('>I', len(value)) + kind + value + struct.pack('>I', zlib.crc32(kind + value))
    rows = b''.join(b'\0' + rgb[y*w*3:(y+1)*w*3] for y in range(h))
    path.write_bytes(b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', w,h,8,2,0,0,0)) + chunk(b'IDAT', zlib.compress(rows)) + chunk(b'IEND', b''))


def inspect(path):
    content = path.read_bytes()
    assert content.startswith(b'\x89PNG\r\n\x1a\n'), path
    w,h = struct.unpack('>II', content[16:24])
    rgb = subprocess.check_output(['ffmpeg','-v','error','-i',str(path),'-f','rawvideo','-pix_fmt','rgb24','-'])
    assert len(rgb) == w*h*3
    stride = w*3
    # Compare the wrap boundary to the distribution of ordinary interior
    # boundaries. Values are absolute differences in 8-bit sRGB, not radiance.
    xbound = [0.0]*(w-1)
    ybound = [0.0]*(h-1)
    xedge = []
    yedge = []
    for y in range(h):
        row=rgb[y*stride:(y+1)*stride]
        xedge.extend(abs(row[c]-row[(w-1)*3+c]) for c in range(3))
        for x in range(w-1):
            p=x*3
            xbound[x] += sum(abs(row[p+c]-row[p+3+c]) for c in range(3))/(h*3)
        if y < h-1:
            nxt=rgb[(y+1)*stride:(y+2)*stride]
            ybound[y] = sum(abs(a-b) for a,b in zip(row,nxt))/stride
    yedge=[abs(a-b) for a,b in zip(rgb[:stride],rgb[-stride:])]

    def stats(edge, inner):
        avg=statistics.mean(edge)
        ordered=sorted(inner)
        med=statistics.median(inner)
        p99=ordered[min(len(ordered)-1,math.ceil(.99*len(ordered))-1)]
        return dict(wrap_mae=avg,interior_boundary_median_mae=med,interior_boundary_p99_mae=p99,wrap_to_interior_median_ratio=avg/med if med else None,wrap_percentile_vs_internal_boundaries=100*sum(v<=avg for v in inner)/len(inner),wrap_channel_p95=sorted(edge)[int(.95*(len(edge)-1))],numeric_screen_pass=avg<=max(2*med,p99))
    xa,ya=stats(xedge,xbound),stats(yedge,ybound)
    review=BASE/'review';review.mkdir(exist_ok=True)
    repeated=b''.join(rgb[y*stride:(y+1)*stride]*2 for y in range(h))*2
    png(review/(path.stem+'-repeat-2x2.png'),w*2,h*2,repeated)
    halfx=w//2;halfy=h//2
    offset=bytearray()
    for y in range(h):
        sy=(y+halfy)%h
        row=rgb[sy*stride:(sy+1)*stride]
        offset.extend(row[halfx*3:]+row[:halfx*3])
    png(review/(path.stem+'-offset.png'),w,h,offset)
    data=dict(file=str(path.relative_to(BASE)),bytes=len(content),sha256=hashlib.sha256(content).hexdigest(),px=[w,h],colour_mode='decoded RGB8',channel_means=[sum(rgb[c::3])/(w*h) for c in range(3)],max_channel_spread=max(max(rgb[i:i+3])-min(rgb[i:i+3]) for i in range(0,len(rgb),3)),seam_metrics={'left_right':xa,'top_bottom':ya},numeric_screen_pass=xa['numeric_screen_pass'] and ya['numeric_screen_pass'],method='Lossless 2x2 repeat and half-width/half-height wrap offset; compare seam MAE against every interior boundary. Numeric screen: each wrap MAE <= max(2x interior median, interior p99). Visual inspection is still required; this screen cannot prove grain continuity or absence of large-scale repetition.',visual_review='pending')
    (review/(path.stem+'-metrics.json')).write_text(json.dumps(data,indent=2)+'\n')
    return data


if __name__=='__main__':
    for name in sys.argv[1:]:
        path=(BASE/name).resolve()
        assert path.is_relative_to(BASE), 'Refuse files outside this lane.'
        print(json.dumps(inspect(path)))
