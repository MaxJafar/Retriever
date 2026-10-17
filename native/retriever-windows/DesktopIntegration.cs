using System.Runtime.InteropServices;
namespace Retriever.Native;
internal sealed class DesktopIntegration:IDisposable {
    [UnmanagedFunctionPointer(CallingConvention.Winapi)]delegate nint Subclass(nint hwnd,uint message,nuint wparam,nint lparam,nuint id,nuint data);
    [StructLayout(LayoutKind.Sequential,CharSet=CharSet.Unicode)]struct NotifyIcon {
        public uint Size;public nint Window;public uint Id,Flags,Callback;public nint Icon;
        [MarshalAs(UnmanagedType.ByValTStr,SizeConst=128)]public string Tip;
        public uint State,StateMask;[MarshalAs(UnmanagedType.ByValTStr,SizeConst=256)]public string Info;public uint Timeout;[MarshalAs(UnmanagedType.ByValTStr,SizeConst=64)]public string Title;public uint InfoFlags;public Guid Guid;public nint Balloon;
    }
    [DllImport("user32.dll")]static extern bool RegisterHotKey(nint hwnd,int id,uint modifiers,uint key);
    [DllImport("user32.dll")]static extern bool UnregisterHotKey(nint hwnd,int id);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)]static extern nint LoadImageW(nint instance,string name,uint type,int width,int height,uint flags);
    [DllImport("user32.dll")]static extern bool DestroyIcon(nint icon);
    [DllImport("comctl32.dll")]static extern bool SetWindowSubclass(nint hwnd,Subclass callback,nuint id,nuint data);
    [DllImport("comctl32.dll")]static extern bool RemoveWindowSubclass(nint hwnd,Subclass callback,nuint id);
    [DllImport("comctl32.dll")]static extern nint DefSubclassProc(nint hwnd,uint message,nuint wparam,nint lparam);
    [DllImport("shell32.dll",CharSet=CharSet.Unicode)]static extern bool Shell_NotifyIconW(uint operation,ref NotifyIcon data);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)]static extern uint RegisterWindowMessageW(string message);
    [DllImport("user32.dll")]static extern nint CreatePopupMenu();
    [DllImport("user32.dll",CharSet=CharSet.Unicode)]static extern bool AppendMenuW(nint menu,uint flags,nuint id,string text);
    [DllImport("user32.dll")]static extern bool DestroyMenu(nint menu);
    [DllImport("user32.dll")]static extern bool GetCursorPos(out Windows.Graphics.PointInt32 point);
    [DllImport("user32.dll")]static extern bool SetForegroundWindow(nint window);
    [DllImport("user32.dll")]static extern uint TrackPopupMenu(nint menu,uint flags,int x,int y,int reserved,nint hwnd,nint rect);
    [DllImport("user32.dll")]static extern bool PostMessageW(nint window,uint message,nuint wparam,nint lparam);
    readonly nint hwnd;readonly Subclass callback;NotifyIcon icon;readonly Action quick;readonly Action show;readonly Action quit;readonly Action health;
    readonly uint taskbarCreated=RegisterWindowMessageW("TaskbarCreated");
    public bool HotkeyRegistered{get;}
    public bool UsesFallback{get;}
    public bool TrayAvailable{get;private set;}
    public DesktopIntegration(nint hwnd,Action quick,Action show,Action quit,Action health){this.health=health;this.hwnd=hwnd;this.quick=quick;this.show=show;this.quit=quit;callback=Handle;SetWindowSubclass(hwnd,callback,731,0);HotkeyRegistered=RegisterHotKey(hwnd,731,0x4000|0x0001|0x0002,0x20);
        if(!HotkeyRegistered){HotkeyRegistered=RegisterHotKey(hwnd,731,0x4000|0x0001|0x0002,0x52);UsesFallback=HotkeyRegistered;}
        icon=new NotifyIcon{Size=(uint)Marshal.SizeOf<NotifyIcon>(),Window=hwnd,Id=731,Flags=1|2|4,Callback=0x8000+731,Tip="Retriever · "+(UsesFallback?"Ctrl+Alt+R":HotkeyRegistered?"Ctrl+Alt+Space":"Quick Search"),Info="",Title="",Icon=LoadImageW(0,Path.Combine(AppContext.BaseDirectory,"Retriever.ico"),1,32,32,0x10)};TrayAvailable=Shell_NotifyIconW(0,ref icon);
    }
    void ShowMenu(){var menu=CreatePopupMenu();try{AppendMenuW(menu,0,1,"Quick Search\t"+(UsesFallback?"Ctrl+Alt+R":"Ctrl+Alt+Space"));AppendMenuW(menu,0,2,"Open Retriever");AppendMenuW(menu,0,4,"Drive health");AppendMenuW(menu,0x800,0,"");AppendMenuW(menu,0,3,"Quit Retriever");GetCursorPos(out var p);SetForegroundWindow(hwnd);var command=TrackPopupMenu(menu,0x100|0x2,p.X,p.Y,0,hwnd,0);PostMessageW(hwnd,0,0,0);if(command==1)quick();else if(command==2)show();else if(command==3)quit();else if(command==4)health();}finally{DestroyMenu(menu);}}
    nint Handle(nint window,uint message,nuint wparam,nint lparam,nuint id,nuint data){if(message==taskbarCreated){TrayAvailable=Shell_NotifyIconW(0,ref icon);return 0;}if(message==0x0312&&wparam==731){quick();return 0;}if(message==0x8000+731){var action=(uint)((long)lparam&0xffff);if(action==0x202)quick();else if(action==0x205)ShowMenu();return 0;}return DefSubclassProc(window,message,wparam,lparam);}
    public void Dispose(){UnregisterHotKey(hwnd,731);RemoveWindowSubclass(hwnd,callback,731);Shell_NotifyIconW(2,ref icon);if(icon.Icon!=0)DestroyIcon(icon.Icon);}
}
