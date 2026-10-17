using System.Runtime.InteropServices;
using System.Text.RegularExpressions;
using Windows.Graphics.Imaging;
using Windows.Storage.Streams;

namespace Retriever.Native;

// Retrieve the current Windows associations using synthetic file names only.
internal static class FileIcons
{
    [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)]
    struct FileInfo {public nint Icon;public int Index;public uint Attributes;[MarshalAs(UnmanagedType.ByValTStr,SizeConst=260)]public string Display;[MarshalAs(UnmanagedType.ByValTStr,SizeConst=80)]public string Type;}
    [StructLayout(LayoutKind.Sequential)]
    struct BitmapHeader {public uint Size;public int Width,Height;public ushort Planes,Bits;public uint Compression,ImageSize;public int X,Y;public uint Used,Important;}
    [DllImport("shell32.dll",CharSet=CharSet.Unicode)]static extern nuint SHGetFileInfoW(string path,uint attributes,ref FileInfo info,uint size,uint flags);
    [DllImport("user32.dll")]static extern bool DestroyIcon(nint icon);
    [DllImport("user32.dll")]static extern bool DrawIconEx(nint dc,int x,int y,nint icon,int width,int height,uint step,nint brush,uint flags);
    [DllImport("gdi32.dll")]static extern nint CreateCompatibleDC(nint dc);
    [DllImport("gdi32.dll")]static extern nint CreateDIBSection(nint dc,ref BitmapHeader info,uint usage,out nint bits,nint section,uint offset);
    [DllImport("gdi32.dll")]static extern nint SelectObject(nint dc,nint obj);
    [DllImport("gdi32.dll")]static extern bool DeleteObject(nint obj);
    [DllImport("gdi32.dll")]static extern bool DeleteDC(nint dc);
    [DllImport("ole32.dll")]static extern int CoInitializeEx(nint reserved,uint mode);
    [DllImport("ole32.dll")]static extern void CoUninitialize();
    static readonly Dictionary<string,object> Cache=new();
    static readonly SemaphoreSlim Gate=new(1,1);
    static (byte[]?,string) Pixels(string key) {
        bool initialized=CoInitializeEx(0,2)>=0;var info=new FileInfo();nint dc=0,bitmap=0,previous=0;
        try {
            string name=key=="folder"?"Retriever folder":"Retriever"+(key=="file"?"":key);
            if(SHGetFileInfoW(name,key=="folder"?0x10u:0x80u,ref info,(uint)Marshal.SizeOf<FileInfo>(),0x100|0x10|0x400)==0||info.Icon==0)return(null,"");
            dc=CreateCompatibleDC(0);var header=new BitmapHeader{Size=(uint)Marshal.SizeOf<BitmapHeader>(),Width=32,Height=-32,Planes=1,Bits=32,ImageSize=4096};
            bitmap=CreateDIBSection(dc,ref header,0,out var bits,0,0);if(dc==0||bitmap==0||bits==0)return(null,"");
            previous=SelectObject(dc,bitmap);var pixels=new byte[4096];Marshal.Copy(pixels,0,bits,pixels.Length);
            if(!DrawIconEx(dc,0,0,info.Icon,32,32,0,0,3))return(null,"");
            Marshal.Copy(bits,pixels,0,pixels.Length);return(pixels,info.Type);
        } finally {if(previous!=0)SelectObject(dc,previous);if(bitmap!=0)DeleteObject(bitmap);if(dc!=0)DeleteDC(dc);if(info.Icon!=0)DestroyIcon(info.Icon);if(initialized)CoUninitialize();}
    }
    internal static async Task<object> Read(string[] extensions) {
        if(extensions.Length>64||extensions.Any(k=>k!="folder"&&k!="file"&&!Regex.IsMatch(k,@"^\.[a-z0-9_+-]{1,24}$")))throw new ArgumentException("Invalid file types");
        await Gate.WaitAsync();
        try {
            var output=new Dictionary<string,object>();
            foreach(var key in extensions.Distinct()) {
                if(!Cache.TryGetValue(key,out var value)) {
                    var (pixels,label)=await Task.Run(()=>Pixels(key));value=new {};
                    if(pixels is not null) {
                        using var stream=new InMemoryRandomAccessStream();var encoder=await BitmapEncoder.CreateAsync(BitmapEncoder.PngEncoderId,stream);
                        encoder.SetPixelData(BitmapPixelFormat.Bgra8,BitmapAlphaMode.Premultiplied,32,32,96,96,pixels);await encoder.FlushAsync();stream.Seek(0);
                        using var reader=new DataReader(stream);await reader.LoadAsync((uint)stream.Size);var bytes=new byte[(int)stream.Size];reader.ReadBytes(bytes);
                        value=new {src="data:image/png;base64,"+Convert.ToBase64String(bytes),label};
                    }
                    if(Cache.Count>=256)Cache.Clear();Cache[key]=value;
                }
                output[key]=value;
            }
            return new {icons=output,platform="Windows"};
        } finally {Gate.Release();}
    }
}

