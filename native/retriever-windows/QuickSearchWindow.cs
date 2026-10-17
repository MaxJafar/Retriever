using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Hosting;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Markup;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Microsoft.UI.Windowing;
using Microsoft.UI.Composition;
using Microsoft.UI.Composition.SystemBackdrops;
using System.Collections.ObjectModel;
using System.Numerics;
using System.Runtime.InteropServices;
using System.Text.Json.Nodes;
using Windows.Graphics;
using Windows.System;
using Windows.UI;
using Windows.UI.ViewManagement;

namespace Retriever.Native;

sealed class QuickSearchBackdrop:SystemBackdrop {
    DesktopAcrylicController? controller;
    protected override void OnTargetConnected(ICompositionSupportsSystemBackdrop target,XamlRoot xamlRoot){
        base.OnTargetConnected(target,xamlRoot);
        if(!DesktopAcrylicController.IsSupported())return;
        controller=new DesktopAcrylicController{Kind=DesktopAcrylicKind.Thin,TintColor=Color.FromArgb(255,24,24,24),TintOpacity=.18f,LuminosityOpacity=.50f,FallbackColor=Color.FromArgb(255,30,30,30)};
        controller.SetSystemBackdropConfiguration(GetDefaultSystemBackdropConfiguration(target,xamlRoot));
        controller.AddSystemBackdropTarget(target);
    }
    protected override void OnTargetDisconnected(ICompositionSupportsSystemBackdrop target){
        controller?.RemoveSystemBackdropTarget(target);controller?.Dispose();controller=null;
        base.OnTargetDisconnected(target);
    }
}

// Independent search surface: never changes the approved Explorer shell.
public sealed class QuickSearchWindow:Window {
    readonly Api api;
    readonly Func<Task<bool>> ready;
    readonly Func<string> localDevice;
    readonly Func<string,string> deviceName;
    readonly Action showMain;
    readonly Func<FileRow,Task> showFile;
    readonly Grid root=new(){RequestedTheme=ElementTheme.Dark};
    readonly TextBox search=new(){PlaceholderText="Search files, or describe what you need…",FontSize=20,BorderThickness=new Thickness(0),Background=new SolidColorBrush(Microsoft.UI.Colors.Transparent),VerticalAlignment=VerticalAlignment.Center};
    readonly ListView results=new(){SelectionMode=ListViewSelectionMode.Single,IsItemClickEnabled=true,Padding=new Thickness(10,0,10,12),Background=new SolidColorBrush(Microsoft.UI.Colors.Transparent)};
    readonly ObservableCollection<FileRow> rows=[];
    readonly StackPanel preview=new(){Spacing=16};
    readonly TextBlock status=new(){Text="Retriever · Quick Search",FontSize=11,Opacity=.65,TextTrimming=TextTrimming.CharacterEllipsis,VerticalAlignment=VerticalAlignment.Center};
    readonly TextBlock count=new(){Text="0",FontSize=10,Opacity=.55,HorizontalAlignment=HorizontalAlignment.Right};
    readonly TextBlock empty=new(){Text="A thought away.\n\nFind a file by name, a phrase, or an idea.",TextAlignment=TextAlignment.Center,TextWrapping=TextWrapping.Wrap,Opacity=.65,FontSize=14,Margin=new Thickness(30),HorizontalAlignment=HorizontalAlignment.Center,VerticalAlignment=VerticalAlignment.Center};
    readonly Button open=new(){Content="Open"},reveal=new(){Content="Show in Retriever"};
    readonly UISettings settings=new();
    CancellationTokenSource? typing,previewRequest;
    long searchRevision,previewRevision,motionRevision;
    bool closing,shown,updatingResults;
    string? previewKey;
    nint previousWindow;
    static readonly SolidColorBrush gold=new(Color.FromArgb(255,229,189,76));
    static readonly SolidColorBrush divider=new(Color.FromArgb(32,255,255,255));
    [DllImport("user32.dll")]static extern nint GetForegroundWindow();
    [DllImport("user32.dll")]static extern bool SetForegroundWindow(nint hwnd);
    [DllImport("user32.dll")]static extern bool GetCursorPos(out PointInt32 point);
    [DllImport("dwmapi.dll")]static extern int DwmSetWindowAttribute(nint hwnd,int attr,ref int value,int size);
    [DllImport("user32.dll",EntryPoint="GetWindowLongPtrW")]static extern nint GetWindowLongPtr(nint hwnd,int index);
    [DllImport("user32.dll",EntryPoint="SetWindowLongPtrW")]static extern nint SetWindowLongPtr(nint hwnd,int index,nint value);
    [DllImport("user32.dll")]static extern bool SetWindowPos(nint hwnd,nint after,int x,int y,int width,int height,uint flags);
    [UnmanagedFunctionPointer(CallingConvention.Winapi)]delegate nint FrameCallback(nint hwnd,uint message,nuint wparam,nint lparam,nuint id,nuint data);
    [StructLayout(LayoutKind.Sequential)]struct WindowStyles {public uint Previous,Next;}
    [DllImport("comctl32.dll")]static extern bool SetWindowSubclass(nint hwnd,FrameCallback callback,nuint id,nuint data);
    [DllImport("comctl32.dll")]static extern bool RemoveWindowSubclass(nint hwnd,FrameCallback callback,nuint id);
    [DllImport("comctl32.dll")]static extern nint DefSubclassProc(nint hwnd,uint message,nuint wparam,nint lparam);
    readonly FrameCallback frameCallback;

    public QuickSearchWindow(Api api,Func<Task<bool>> ready,Func<string> localDevice,Func<string,string> deviceName,Action showMain,Func<FileRow,Task> showFile) {
        this.api=api;this.ready=ready;this.localDevice=localDevice;this.deviceName=deviceName;this.showMain=showMain;this.showFile=showFile;
        frameCallback=KeepBorderless;
        SetWindowSubclass(WinRT.Interop.WindowNative.GetWindowHandle(this),frameCallback,732,0);
        Title="Retriever Quick Search — Candidate";
        SystemBackdrop=new QuickSearchBackdrop();
        var presenter=OverlappedPresenter.Create();presenter.IsAlwaysOnTop=false;presenter.IsResizable=false;presenter.IsMaximizable=false;presenter.IsMinimizable=false;presenter.SetBorderAndTitleBar(false,false);AppWindow.SetPresenter(presenter);
        AppWindow.IsShownInSwitchers=false;
        AppWindow.Resize(new SizeInt32(940,580));
        ApplyFrame();
        Build();Content=root;
        results.ItemsSource=rows;
        results.SelectionChanged+=async(_,_)=>{if(!updatingResults)await PreviewSelection();};
        results.DoubleTapped+=async(_,_)=>await Open();
        results.KeyDown+=async(_,e)=>{if(e.Key==VirtualKey.Enter){e.Handled=true;await Open();}};
        search.TextChanged+=(_,_)=>QueueSearch();
        search.PreviewKeyDown+=async(_,e)=>{
            if(e.Key is VirtualKey.Down or VirtualKey.Up){e.Handled=true;Move(e.Key==VirtualKey.Down?1:-1);}
            else if(e.Key==VirtualKey.Enter){e.Handled=true;if(results.SelectedItem!=null)await Open();else QueueSearch(immediate:true);}
        };
        root.PreviewKeyDown+=(_,e)=>{if(e.Key==VirtualKey.Escape){e.Handled=true;Hide();}};
        var focus=new KeyboardAccelerator{Key=VirtualKey.K,Modifiers=VirtualKeyModifiers.Control};focus.Invoked+=(_,e)=>{search.Focus(FocusState.Keyboard);search.SelectAll();e.Handled=true;};root.KeyboardAccelerators.Add(focus);
        open.Click+=async(_,_)=>await Open();reveal.Click+=async(_,_)=>await Reveal();
        AppWindow.Closing+=(_,e)=>{if(!closing){e.Cancel=true;Hide();}};
        Activated+=(_,e)=>{if(e.WindowActivationState==WindowActivationState.Deactivated){if(shown)Hide(false);}else ApplyFrame();};
        root.Loaded+=(_,_)=>{ApplyFrame();if(shown){search.Focus(FocusState.Programmatic);Animate(true);}};
        Closed+=(_,_)=>{CancelWork();RemoveWindowSubclass(WinRT.Interop.WindowNative.GetWindowHandle(this),frameCallback,732);};
    }
    nint KeepBorderless(nint window,uint message,nuint wparam,nint lparam,nuint id,nuint data){
        // WinAppSDK 1.6 reapplies WS_DLGFRAME when a fixed-size presenter changes.
        // Preserve the borderless policy across those native style updates.
        var result=DefSubclassProc(window,message,wparam,lparam);
        if(message==0x007c&&unchecked((int)wparam)==-16&&lparam!=0){var styles=Marshal.PtrToStructure<WindowStyles>(lparam);styles.Next&=~0x00400000u;Marshal.StructureToPtr(styles,lparam,false);}
        return result;
    }
    void ApplyFrame(){
        if(AppWindow.Presenter is OverlappedPresenter presenter)presenter.SetBorderAndTitleBar(false,false);
        var handle=WinRT.Interop.WindowNative.GetWindowHandle(this);
        // The presenter can retain a classic client edge on a borderless HWND.
        // Remove those frame styles as well as the compositor's colored border.
        var style=GetWindowLongPtr(handle,-16).ToInt64();
        var extended=GetWindowLongPtr(handle,-20).ToInt64();
        SetWindowLongPtr(handle,-16,(nint)((style&~0x00c40000L)|0x80000000L));
        SetWindowLongPtr(handle,-20,(nint)(extended&~0x00020300L));
        SetWindowPos(handle,0,0,0,0,0,0x0037);
        var dark=1;DwmSetWindowAttribute(handle,20,ref dark,4);
        var rounded=2;DwmSetWindowAttribute(handle,33,ref rounded,4);
        var noBorder=unchecked((int)0xfffffffe);DwmSetWindowAttribute(handle,34,ref noBorder,4);
    }
    static TextBlock Label(string text,double size=12)=>new(){Text=text,FontSize=size,TextWrapping=TextWrapping.Wrap};
    static Button Plain(string text,Action click){var b=new Button{Content=text,Background=new SolidColorBrush(Microsoft.UI.Colors.Transparent),BorderThickness=new Thickness(0),FontSize=11};b.Click+=(_,_)=>click();return b;}
    static Border Rule(UIElement child)=>new(){Child=child,BorderBrush=divider,BorderThickness=new Thickness(0,0,0,1)};
    void Build(){
        // Match Windows' compositor-rounded corners without a second rectangular
        // focus outline. The native acrylic supplies blur and accessibility fallback.
        root.Background=new SolidColorBrush(Color.FromArgb(DesktopAcrylicController.IsSupported()?(byte)24:(byte)255,24,24,24));root.CornerRadius=new CornerRadius(8);root.BorderThickness=new Thickness(0);
        root.Resources["ListViewItemBackgroundSelected"]=new SolidColorBrush(Color.FromArgb(50,229,189,76));
        root.Resources["ListViewItemBackgroundSelectedPointerOver"]=new SolidColorBrush(Color.FromArgb(65,229,189,76));
        root.Resources["ListViewItemBackgroundPointerOver"]=new SolidColorBrush(Color.FromArgb(16,255,255,255));
        root.Resources["ListViewItemSelectionIndicatorBrush"]=gold;
        root.RowDefinitions.Add(new(){Height=new GridLength(80)});root.RowDefinitions.Add(new(){Height=new GridLength(1,GridUnitType.Star)});root.RowDefinitions.Add(new(){Height=new GridLength(38)});
        var header=new Grid{Padding=new Thickness(23,0,18,0),ColumnSpacing=14};foreach(var width in new[]{new GridLength(34),new GridLength(22),new GridLength(1,GridUnitType.Star),GridLength.Auto})header.ColumnDefinitions.Add(new(){Width=width});
        var logo=new Image{Source=new BitmapImage(new Uri(Path.Combine(AppContext.BaseDirectory,"Retriever.png"))),Width=34,Height=34};header.Children.Add(logo);
        var magnifier=new FontIcon{Glyph="\uE721",FontSize=21,Foreground=gold};Grid.SetColumn(magnifier,1);header.Children.Add(magnifier);
        search.Resources["TextControlBackground"]=new SolidColorBrush(Microsoft.UI.Colors.Transparent);search.Resources["TextControlBackgroundFocused"]=new SolidColorBrush(Microsoft.UI.Colors.Transparent);search.Resources["TextControlBorderBrushFocused"]=new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        search.Resources["TextControlBorderThemeThicknessFocused"]=new Thickness(0);search.UseSystemFocusVisuals=false;
        AutomationProperties.SetName(search,"Search your files");Grid.SetColumn(search,2);header.Children.Add(search);
        var escape=Plain("esc",()=>Hide());AutomationProperties.SetName(escape,"Close Quick Search");Grid.SetColumn(escape,3);header.Children.Add(escape);root.Children.Add(Rule(header));
        var body=new Grid();body.ColumnDefinitions.Add(new(){Width=new GridLength(53,GridUnitType.Star)});body.ColumnDefinitions.Add(new(){Width=new GridLength(47,GridUnitType.Star)});Grid.SetRow(body,1);root.Children.Add(body);
        var left=new Grid{Background=new SolidColorBrush(Color.FromArgb(14,0,0,0)),BorderBrush=divider,BorderThickness=new Thickness(0,0,1,0)};left.RowDefinitions.Add(new(){Height=new GridLength(36)});left.RowDefinitions.Add(new(){Height=new GridLength(1,GridUnitType.Star)});
        var heading=new Grid{Margin=new Thickness(22,10,22,0)};var title=Label("RESULTS",10);title.CharacterSpacing=140;title.Opacity=.55;heading.Children.Add(title);heading.Children.Add(count);left.Children.Add(heading);
        results.ItemTemplate=(DataTemplate)XamlReader.Load("<DataTemplate xmlns='http://schemas.microsoft.com/winfx/2006/xaml/presentation'><Grid Height='62' Padding='8,0' ColumnSpacing='12'><Grid.ColumnDefinitions><ColumnDefinition Width='28'/><ColumnDefinition Width='*'/></Grid.ColumnDefinitions><Grid><FontIcon Glyph='{Binding Glyph}' FontSize='24' Foreground='#E5BD4C'/><Image Source='{Binding Icon}' Width='28' Height='28'/></Grid><StackPanel Grid.Column='1' VerticalAlignment='Center' Spacing='4'><TextBlock Text='{Binding Name}' FontSize='13' FontWeight='SemiBold' TextTrimming='CharacterEllipsis'/><TextBlock Text='{Binding Detail}' FontSize='11' Opacity='.6' TextTrimming='CharacterEllipsis'/></StackPanel></Grid></DataTemplate>");
        results.ContainerContentChanging+=(_,e)=>{if(e.Item is FileRow row)AutomationProperties.SetName(e.ItemContainer,row.Name);};Grid.SetRow(results,1);left.Children.Add(results);Grid.SetRow(empty,1);left.Children.Add(empty);body.Children.Add(left);
        var right=new Grid{Padding=new Thickness(22)};right.RowDefinitions.Add(new(){Height=new GridLength(25)});right.RowDefinitions.Add(new(){Height=new GridLength(1,GridUnitType.Star)});right.RowDefinitions.Add(new(){Height=GridLength.Auto});var look=Label("QUICK LOOK",10);look.CharacterSpacing=140;look.Opacity=.55;right.Children.Add(look);
        var scroll=new ScrollViewer{Content=preview,HorizontalScrollBarVisibility=ScrollBarVisibility.Disabled,VerticalScrollBarVisibility=ScrollBarVisibility.Auto};Grid.SetRow(scroll,1);right.Children.Add(scroll);
        var actions=new StackPanel{Orientation=Orientation.Horizontal,Spacing=10,Margin=new Thickness(0,12,0,0)};open.Background=gold;open.Foreground=new SolidColorBrush(Microsoft.UI.Colors.Black);open.IsEnabled=false;reveal.IsEnabled=false;actions.Children.Add(open);actions.Children.Add(reveal);Grid.SetRow(actions,2);right.Children.Add(actions);Grid.SetColumn(right,1);body.Children.Add(right);
        var footer=new Grid{Padding=new Thickness(22,0,18,0),BorderBrush=divider,BorderThickness=new Thickness(0,1,0,0),ColumnSpacing=16};footer.ColumnDefinitions.Add(new(){Width=new GridLength(1,GridUnitType.Star)});footer.ColumnDefinitions.Add(new(){Width=GridLength.Auto});footer.Children.Add(status);var hints=new StackPanel{Orientation=Orientation.Horizontal,Spacing=16,VerticalAlignment=VerticalAlignment.Center};hints.Children.Add(Label("↑ ↓ Select    ↵ Open",11));var main=Plain("Open Retriever",()=>{Hide(false);showMain();});main.Foreground=gold;hints.Children.Add(main);Grid.SetColumn(hints,1);footer.Children.Add(hints);Grid.SetRow(footer,2);root.Children.Add(footer);
        EmptyPreview();
    }
    void EmptyPreview(){preview.Children.Clear();preview.Children.Add(new TextBlock{Text="A little more context.\n\nSelect a result to preview it here.",FontSize=14,Opacity=.6,TextAlignment=TextAlignment.Center,TextWrapping=TextWrapping.Wrap,Margin=new Thickness(12,95,12,12)});}
    public void Toggle(){if(shown)Hide();else Show();}
    public void Show(){
        previousWindow=GetForegroundWindow();shown=true;Interlocked.Increment(ref motionRevision);
        if(GetCursorPos(out var point)){var area=DisplayArea.GetFromPoint(point,DisplayAreaFallback.Nearest).WorkArea;var width=Math.Min(940,area.Width-48);var height=Math.Min(580,area.Height-64);AppWindow.MoveAndResize(new RectInt32(area.X+(area.Width-width)/2,area.Y+(area.Height-height)/2-35,width,height));}
        Activate();SetForegroundWindow(WinRT.Interop.WindowNative.GetWindowHandle(this));search.Focus(FocusState.Programmatic);search.SelectAll();Animate(true);if(search.Text.Length>0)QueueSearch();
        DispatcherQueue.TryEnqueue(()=>{if(shown){ApplyFrame();search.Focus(FocusState.Programmatic);search.SelectAll();}});
    }
    public async void Hide(bool restoreFocus=true){
        if(!shown)return;shown=false;CancelWork();var revision=Interlocked.Increment(ref motionRevision);Animate(false);if(settings.AnimationsEnabled)await Task.Delay(120);if(revision!=motionRevision)return;AppWindow.Hide();if(restoreFocus&&previousWindow!=0)SetForegroundWindow(previousWindow);
    }
    void Animate(bool visible){
        var visual=ElementCompositionPreview.GetElementVisual(root);visual.StopAnimation("Opacity");visual.StopAnimation("Scale");
        visual.Opacity=visible?1:0;visual.Scale=Vector3.One;
        if(!settings.AnimationsEnabled)return;
        var compositor=visual.Compositor;var easing=compositor.CreateCubicBezierEasingFunction(new Vector2(.16f,1),new Vector2(.3f,1));
        var opacity=compositor.CreateScalarKeyFrameAnimation();opacity.InsertKeyFrame(0,visible?0:1);opacity.InsertKeyFrame(1,visible?1:0,easing);opacity.Duration=TimeSpan.FromMilliseconds(visible?180:120);visual.StartAnimation("Opacity",opacity);
        if(visible){visual.CenterPoint=new Vector3((float)root.ActualWidth/2,(float)root.ActualHeight/2,0);var scale=compositor.CreateVector3KeyFrameAnimation();scale.InsertKeyFrame(0,new Vector3(.975f,.975f,1));scale.InsertKeyFrame(1,Vector3.One,easing);scale.Duration=TimeSpan.FromMilliseconds(220);visual.StartAnimation("Scale",scale);}
    }
    void CancelWork(){typing?.Cancel();previewRequest?.Cancel();Interlocked.Increment(ref searchRevision);Interlocked.Increment(ref previewRevision);}
    void Move(int delta){if(rows.Count==0)return;results.SelectedIndex=Math.Clamp(results.SelectedIndex<0?(delta>0?0:rows.Count-1):results.SelectedIndex+delta,0,rows.Count-1);results.ScrollIntoView(results.SelectedItem);}
    async void QueueSearch(bool immediate=false){
        typing?.Cancel();typing?.Dispose();typing=new();var token=typing.Token;var revision=Interlocked.Increment(ref searchRevision);var query=search.Text.Trim();
        // Drop stale results immediately: Enter can never open a hit from an older query.
        rows.Clear();count.Text="0";empty.Visibility=Visibility.Visible;empty.Text=query.Length==0?"A thought away.\n\nFind a file by name, a phrase, or an idea.":"Searching…";
        if(query.Length==0){status.Text="Retriever · Quick Search";return;}
        try {
            if(!await ready()){status.Text="The coordinator is unavailable. Open Retriever to reconnect.";return;}
            token.ThrowIfCancellationRequested();if(!immediate)await Task.Delay(180,token);
            await Search(query,immediate,revision,token);
            if(!immediate){await Task.Delay(420,token);await Search(query,true,revision,token);}
        }catch(OperationCanceledException){}catch(Exception e){if(revision==searchRevision){status.Text=e.Message;empty.Text="Search unavailable. Check your connection in Retriever.";}}
    }
    async Task Search(string query,bool full,long revision,CancellationToken token){
        {
            var response=await api.Get($"search?q={Uri.EscapeDataString(query)}&phase={(full?"full":"instant")}",token);
            if(revision!=searchRevision||query!=search.Text.Trim()||!shown)return;
            var selected=results.SelectedItem as FileRow;var selectedKey=selected?.Device+":"+selected?.Path;
            updatingResults=true;
            try {
                rows.Clear();foreach(var item in response["results"]?.AsArray()??[])if(item!=null)rows.Add(new FileRow(item,deviceName(item["device"]?.ToString()??"")));
                if(rows.Count>0)results.SelectedItem=rows.FirstOrDefault(r=>r.Device+":"+r.Path==selectedKey)??rows[0];
            }finally{updatingResults=false;}
            count.Text=rows.Count.ToString();empty.Visibility=rows.Count==0?Visibility.Visible:Visibility.Collapsed;empty.Text="No matches yet.\n\nTry a filename or a few different words.";
            _=PreviewSelection();
            status.Text=$"{rows.Count} matches · {response["elapsed_ms"]?.GetValue<double>():0.0} ms";
        }
    }
    async Task PreviewSelection(){
        var next=results.SelectedItem as FileRow;
        var key=next==null?null:next.Device+":"+next.Path+":"+next.Data["modified"]+":"+next.Data["digest"];
        if(key!=null&&key==previewKey&&previewRequest?.IsCancellationRequested==false)return;
        previewKey=key;
        previewRequest?.Cancel();previewRequest?.Dispose();previewRequest=new();var token=previewRequest.Token;var revision=Interlocked.Increment(ref previewRevision);
        open.IsEnabled=false;reveal.IsEnabled=false;
        if(results.SelectedItem is not FileRow file){EmptyPreview();return;}
        preview.Children.Clear();preview.Children.Add(Label(file.Name,17));preview.Children.Add(Label("Loading preview…",12));
        open.IsEnabled=true;reveal.IsEnabled=true;
        try{
            await Task.Delay(160,token);
            JsonNode response=new JsonObject();
            for(int attempt=0;attempt<5;attempt++){
                response=await api.Post("workspace/preview",new JsonObject{["device"]=file.Device,["path"]=file.Path,["visual"]=true},cancel:token);
                if(response["visual_pending"]?.GetValue<bool>()!=true)break;
                await Task.Delay(300+attempt*150,token);
            }
            if(revision!=previewRevision||token.IsCancellationRequested)return;
            var content=new List<UIElement>();var uri=response["image"]?.ToString();
            if(uri?.StartsWith("data:image/jpeg;base64,")==true){
                var bytes=Convert.FromBase64String(uri[23..]);using var stream=new Windows.Storage.Streams.InMemoryRandomAccessStream();using(var writer=new Windows.Storage.Streams.DataWriter(stream)){writer.WriteBytes(bytes);await writer.StoreAsync();writer.DetachStream();}stream.Seek(0);var bitmap=new BitmapImage();await bitmap.SetSourceAsync(stream);content.Add(new Image{Source=bitmap,MaxHeight=230,Stretch=Stretch.Uniform});
            }else{var icon=new Image{Width=56,Height=56,Margin=new Thickness(0,16,0,0)};icon.SetBinding(Image.SourceProperty,new Microsoft.UI.Xaml.Data.Binding{Source=file,Path=new PropertyPath("Icon")});content.Add(icon);}
            content.Add(new TextBlock{Text=file.Name,FontSize=17,FontWeight=Microsoft.UI.Text.FontWeights.SemiBold,TextWrapping=TextWrapping.Wrap,IsTextSelectionEnabled=true});
            content.Add(new TextBlock{Text=file.Path,FontSize=11,Opacity=.6,MaxLines=3,TextWrapping=TextWrapping.Wrap,IsTextSelectionEnabled=true});
            content.Add(new TextBlock{Text=$"{file.TypeLabel} · {file.Size} · {file.DeviceLabel}",FontSize=11,Opacity=.6,TextWrapping=TextWrapping.Wrap});
            if(response["coverage"]?.ToString() is string coverage)content.Add(new TextBlock{Text=coverage,FontSize=10,Foreground=gold});
            if(response["offline"]?.GetValue<bool>()==true)content.Add(Label("Source offline · showing indexed text",11));else if(response["visual_error"]?.ToString() is string warning)content.Add(Label(warning,11));
            var text=response["text"]?.ToString();if(!string.IsNullOrWhiteSpace(text))content.Add(new TextBlock{Text=text,FontSize=12,TextWrapping=TextWrapping.Wrap,IsTextSelectionEnabled=true});
            if(revision!=previewRevision||token.IsCancellationRequested)return;preview.Children.Clear();foreach(var item in content)preview.Children.Add(item);
        }catch(OperationCanceledException){}catch(Exception e){if(revision==previewRevision){preview.Children.Clear();preview.Children.Add(Label(file.Name,17));preview.Children.Add(Label(e.Message));}}
    }
    async Task Open(){
        if(results.SelectedItem is not FileRow file)return;
        try{
            if(api.Mode=="client"||file.Device!=localDevice())throw new InvalidOperationException("Open this file on its source device. Remote opening is not available yet.");
            if(file.Kind=="folder"){Hide(false);await showFile(file);return;}
            bool opened=await Launcher.LaunchFileAsync(await Windows.Storage.StorageFile.GetFileFromPathAsync(file.Path));
            if(opened)Hide(false);else status.Text="Windows could not open this file.";
        }catch(Exception e){status.Text=e.Message;}
    }
    async Task Reveal(){
        if(results.SelectedItem is not FileRow file)return;
        try{Hide(false);await showFile(file);}catch(Exception e){status.Text=e.Message;}
    }
    public void Shutdown(){closing=true;CancelWork();Close();}
}
