using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using System.Text.Json.Nodes;

namespace Retriever.Native;

public sealed partial class MainWindow {
    Task DesktopPopover(){
        var body=new StackPanel{Spacing=12};
        body.Children.Add(new TextBlock{Text="Retriever for Windows",FontSize=17,FontWeight=Microsoft.UI.Text.FontWeights.SemiBold});
        body.Children.Add(new TextBlock{Text="Close the window to keep Retriever in the tray. Quick Search stays available in other apps.",TextWrapping=TextWrapping.Wrap,FontSize=12,Opacity=.7});
        body.Children.Add(Label(desktop?.HotkeyRegistered==true?(desktop.UsesFallback?"Ctrl + Alt + R · Quick Search":"Ctrl + Alt + Space · Quick Search"):"Shortcut unavailable · use the tray icon"));
        body.Children.Add(Label(desktop?.TrayAvailable==true?"● Tray active":"Tray unavailable",11));
        body.Children.Add(FlatButton("Open Quick Search","\uE721",()=>{activePopover?.Hide();ToggleQuick();return Task.CompletedTask;}));
        body.Children.Add(FlatButton("Connection settings","\uE968",async()=>{var anchor=lastActionAnchor;await ClosePopover();await ConnectionPopover(anchor);}));
        body.Children.Add(FlatButton("Folder integration","\uE8B7",async()=>{await ClosePopover();await FolderIntegrationPopover();}));
        Popover("Desktop app",body,width:330);return Task.CompletedTask;
    }
    Task ConnectionPopover(FrameworkElement? anchor=null){
        var mode=new ComboBox{Header="Application mode",ItemsSource=new[]{"This computer is the coordinator","Connect to another coordinator"},SelectedIndex=api.Mode=="client"?1:0,HorizontalAlignment=HorizontalAlignment.Stretch};
        var endpoint=new TextBox{Header="Coordinator URL",Text=api.Endpoint};var token=new PasswordBox{Header="Pairing token",Password=api.Mode=="client"?api.Token:""};var error=new TextBlock{TextWrapping=TextWrapping.Wrap,FontSize=11,Foreground=Brush(0xe5a088)};
        var body=new StackPanel{Spacing=10};foreach(var element in new UIElement[]{mode,endpoint,token,error})body.Children.Add(element);
        void State(){endpoint.Visibility=token.Visibility=mode.SelectedIndex==1?Visibility.Visible:Visibility.Collapsed;}mode.SelectionChanged+=(_,_)=>State();State();
        var connect=FlatButton("Connect","\uE73E",async()=>{try{if(mode.SelectedIndex==0)await api.UseCoordinatorAsync();else await api.SaveClientAsync(endpoint.Text,token.Password);activePopover?.Hide();await RefreshConfig();await Navigate("files");}catch(Exception e){error.Text=e.Message;}});body.Children.Add(connect);
        body.Children.Add(new TextBlock{Text="Use HTTPS or a local SSH tunnel. Credentials are protected for your Windows account.",TextWrapping=TextWrapping.Wrap,Opacity=.55,FontSize=10});Popover("Connection settings",body,anchor,340);return Task.CompletedTask;
    }
    async Task ConnectionsPopover(){
        var data=await api.Get("status");var body=new StackPanel{Spacing=10};
        foreach(var device in config?["devices"]?.AsArray()??[]){if(device==null)continue;var id=device["id"]!.ToString();var remote=device["transport"]?.ToString()=="ssh";var entry=(data["devices"]?.AsArray()??[]).FirstOrDefault(d=>d?["id"]?.ToString()==id);var stack=new StackPanel{Spacing=5};stack.Children.Add(Label(device["label"]?.ToString()??id,13));var detail=new TextBlock{Text=remote?"SSH · "+(entry?["status"]?.ToString()??"Not checked"):"Local coordinator",FontSize=11,Opacity=.65,TextWrapping=TextWrapping.Wrap};stack.Children.Add(detail);
            var connectionBar=new ProgressBar{Height=4,Maximum=1,Value=remote?0:1,Foreground=accent};stack.Children.Add(connectionBar);
            if(remote)stack.Children.Add(FlatButton("Check connection","\uE72C",async()=>{detail.Text="Connecting…";connectionBar.IsIndeterminate=true;try{var probe=await api.Post("device-probe",new JsonObject{["device"]=id});detail.Text="Connected · "+probe["platform"]+" · "+probe["engine_version"];connectionBar.Value=1;}catch(Exception ex){detail.Text="Unavailable · "+ex.Message;connectionBar.Value=0;}finally{connectionBar.IsIndeterminate=false;}}));
            stack.Children.Add(FlatButton("Browse files","\uE8B7",async()=>{await ClosePopover();explorerLocations[id]=await DeviceLocations(id);collapsedDevices.Remove(id);SaveFolderTree();RenderFolderTree();if(explorerLocations[id].FirstOrDefault()?["path"] is JsonNode path)await Browse(id,path.ToString());}));
            if(remote)stack.Children.Add(FlatButton("Storage snapshot","\uEDA2",async()=>{await ClosePopover();await RemoteStoragePopover(id);}));body.Children.Add(InsightCard(remote?"Remote device":"This computer",stack));
        }
        body.Children.Add(FlatButton("Discover Tailscale devices","\uE968",async()=>{await ClosePopover();await DiscoverNodes();}));
        body.Children.Add(FlatButton("Add remote device","\uE710",async()=>{await ClosePopover();await AddRemoteDevice();}));
        body.Children.Add(FlatButton("Coordinator settings","\uE713",async()=>{var anchor=lastActionAnchor;await ClosePopover();await ConnectionPopover(anchor);}));
        body.Children.Add(new TextBlock{Text="Connection checks only contact the Retriever agent. Disk health collection remains blocked.",FontSize=10,Opacity=.55,TextWrapping=TextWrapping.Wrap});Popover("Connections",new ScrollViewer{Content=body,MaxHeight=460},width:350);
    }
    async Task ActivityPopover(){
        var data=await api.Get("workspace");var body=new StackPanel{Spacing=10};var history=data["history"]?.AsArray()??[];
        if(history.Count==0)body.Children.Add(new TextBlock{Text="You're all caught up. File operations will appear here.",TextWrapping=TextWrapping.Wrap,Opacity=.6});
        foreach(var item in history.Take(12)){if(item==null)continue;var row=new StackPanel{Spacing=4};row.Children.Add(Label($"{item["operation"]} · {item["state"]}",12));if(double.TryParse(item["created"]?.ToString(),out var seconds))row.Children.Add(new TextBlock{Text=DateTimeOffset.FromUnixTimeSeconds((long)seconds).ToLocalTime().ToString("g"),FontSize=10,Opacity=.55});body.Children.Add(row);}
        body.Children.Add(FlatButton("Full history & undo","\uE81C",async()=>{activePopover?.Hide();await Activity();}));Popover("Activity",new ScrollViewer{Content=body,MaxHeight=420},width:320);
    }
}
