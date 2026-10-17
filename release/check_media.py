"""Exercise packaged decoders on tiny generated media before making an installer."""
import argparse
import ctypes
import json
import os
from pathlib import Path
import subprocess
import tempfile

def check(engine, platform, output):
    output.mkdir(parents=True, exist_ok=True)
    suffix = '.exe' if platform == 'windows-x64' else ''
    ffmpeg = str(engine / ('ffmpeg' + suffix))
    ffprobe = str(engine / ('ffprobe' + suffix))
    def run(*args):
        return subprocess.check_output([ffmpeg, '-hide_banner', '-loglevel', 'error', '-nostdin', *args], stderr=subprocess.PIPE, timeout=30)
    with tempfile.TemporaryDirectory(dir=output) as tmp:
        movie = str(Path(tmp) / 'sample.mp4')
        png = str(Path(tmp) / 'sample.png')
        run('-f', 'lavfi', '-i', 'color=c=blue:s=32x32:d=0.12', '-c:v', 'mpeg4', '-an', '-n', movie)
        metadata = json.loads(subprocess.check_output([ffprobe, '-v', 'error', '-show_streams', '-of', 'json', movie], timeout=30))
        assert metadata['streams'][0]['width'] == 32
        jpeg = run('-i', movie, '-frames:v', '1', '-f', 'image2pipe', '-vcodec', 'mjpeg', 'pipe:1')
        assert jpeg.startswith(b'\xff\xd8') and jpeg.endswith(b'\xff\xd9')
        run('-i', movie, '-frames:v', '1', '-n', png)
        pixels = run('-i', png, '-frames:v', '1', '-f', 'rawvideo', '-pix_fmt', 'rgb24', 'pipe:1')
        assert len(pixels) == 32 * 32 * 3
    # Load the shipped library and parse an actual one-page PDF in memory.
    objects = [b'<< /Type /Catalog /Pages 2 0 R >>', b'<< /Type /Pages /Kids [3 0 R] /Count 1 >>', b'<< /Type /Page /Parent 2 0 R /MediaBox [0 0 32 32] /Resources <<>> >>']
    pdf = bytearray(b'%PDF-1.4\n'); offsets = [0]
    for i, obj in enumerate(objects, 1):
        offsets.append(len(pdf)); pdf += f'{i} 0 obj\n'.encode() + obj + b'\nendobj\n'
    xref = len(pdf); pdf += b'xref\n0 4\n0000000000 65535 f \n'
    for offset in offsets[1:]: pdf += f'{offset:010} 00000 n \n'.encode()
    pdf += f'trailer\n<< /Size 4 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n'.encode()
    library = ctypes.CDLL(str(engine / ('pdfium.dll' if platform == 'windows-x64' else 'libpdfium.dylib')))
    library.FPDF_InitLibrary.argtypes = []; library.FPDF_InitLibrary.restype = None
    library.FPDF_DestroyLibrary.argtypes = []; library.FPDF_DestroyLibrary.restype = None
    library.FPDF_LoadMemDocument64.argtypes = [ctypes.c_void_p, ctypes.c_size_t, ctypes.c_char_p]; library.FPDF_LoadMemDocument64.restype = ctypes.c_void_p
    library.FPDF_GetPageCount.argtypes = [ctypes.c_void_p]; library.FPDF_GetPageCount.restype = ctypes.c_int
    library.FPDF_CloseDocument.argtypes = [ctypes.c_void_p]; library.FPDF_CloseDocument.restype = None
    buffer = ctypes.create_string_buffer(bytes(pdf)); document = None
    library.FPDF_InitLibrary()
    try:
        document = library.FPDF_LoadMemDocument64(buffer, len(pdf), None)
        assert document and library.FPDF_GetPageCount(document) == 1
    finally:
        if document: library.FPDF_CloseDocument(document)
        library.FPDF_DestroyLibrary()
    report = {'passed': True, 'platform': platform, 'checks': ['video-probe', 'video-jpeg-preview', 'png-encode-decode', 'pdf-load'], 'fixture_only': True}
    (output / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    return report

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--engine', type=Path, required=True)
    parser.add_argument('--platform', choices=['windows-x64', 'macos-arm64'], required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(check(args.engine.resolve(), args.platform, args.output.resolve())))
