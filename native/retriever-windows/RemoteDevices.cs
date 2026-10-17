using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using System.Text.Json.Nodes;

namespace Retriever.Native;

public sealed partial class MainWindow {
    async Task DiscoverNodes(){
        var body=new StackPanel{Spacing=10};var data=await api.Get("nodes/discover");
        foreach(var node in data["nodes"]?.AsArray()??[]){if(node==null)continue;var row=new StackPanel{Spacing=3};row.Children.Add(Label(node["name"]?.ToString()??"Device",13));row.Children.Add(new TextBlock{Text=$"{node["address"]} · {node["platform"]} · {(node["online"]?.GetValue<bool>()==true?"Online":"Offline")}{(node["local"]?.GetValue<bool>()==true?" · This computer":"")}",FontSize=11,Opacity=.65});body.Children.Add(row);}
        body.Children.Add(new TextBlock{Text="Online means reachable on Tailscale. Pair a saved SSH alias and matching Retriever agent to browse files. Discovery does not grant access or start crawling.",TextWrapping=TextWrapping.Wrap,FontSize=11,Opacity=.65});
        body.Children.Add(FlatButton("Connect a device","\uE710",async()=>{await ClosePopover();await AddRemoteDevice();}));Popover("Lab devices",new ScrollViewer{Content=body,MaxHeight=470},width:380);
    }
    Task AddRemoteDevice(){
        var body=new StackPanel{Spacing=10};
        body.Children.Add(new TextBlock{Text="Connect through an existing SSH alias. The headless agent starts only when requested; it needs no model server or background service.",TextWrapping=TextWrapping.Wrap,FontSize=12,Opacity=.7});
        var label=new TextBox{Header="Device name",PlaceholderText="MacBook Pro"};var alias=new TextBox{Header="Saved SSH alias",PlaceholderText="my-macbook"};
        var platform=new ComboBox{Header="Operating system",ItemsSource=new[]{"macos","windows","linux"},SelectedIndex=0,HorizontalAlignment=HorizontalAlignment.Stretch};
        var agent=new TextBox{Header="Agent executable on that device",PlaceholderText="/Users/name/.local/bin/retriever-agent",Text="retriever-agent"};
        foreach(var field in new UIElement[]{label,alias,platform,agent})body.Children.Add(field);
        body.Children.Add(new TextBlock{Text="Install the matching headless package on the device, verify its host key, then save its SSH alias in your SSH configuration. The installer prints the agent path. No passwords are stored here.",FontSize=11,TextWrapping=TextWrapping.Wrap,Opacity=.6});
        var feedback=new TextBlock{TextWrapping=TextWrapping.Wrap,FontSize=11};body.Children.Add(feedback);
        Button? save=null;save=FlatButton("Add device","\uE710",async()=>{save!.IsEnabled=false;try{var updated=config!.DeepClone();var id="remote-"+Guid.NewGuid().ToString("N")[..12];updated["devices"]!.AsArray().Add(new JsonObject{["id"]=id,["label"]=label.Text.Trim(),["transport"]="ssh",["platform"]=platform.SelectedItem!.ToString(),["enabled"]=true,["ssh"]=alias.Text.Trim(),["agent"]=agent.Text.Trim(),["roots"]=new JsonArray(),["exclude"]=new JsonArray(),["analysis_roots"]=new JsonArray()});if(string.IsNullOrWhiteSpace(label.Text))throw new InvalidOperationException("Give this device a name.");await api.Post("config",updated,revision);await RefreshConfig();activePopover?.Hide();message.Text="Device added. Check its connection in Connections; AI crawling is off.";}catch(Exception e){feedback.Text=e.Message;}finally{save.IsEnabled=true;}});body.Children.Add(save);Popover("Add remote device",new ScrollViewer{Content=body,MaxHeight=510},width:360);return Task.CompletedTask;
    }
    Task RemoteStoragePopover(string device){
        var body=new StackPanel{Spacing=12};var folder=new TextBox{Header="Folder on "+DeviceName(device),PlaceholderText="Absolute folder path"};body.Children.Add(folder);
        body.Children.Add(new TextBlock{Text="A bounded directory snapshot over SSH. Reads file metadata only; no models or drive health probes. Large folders may return partial totals.",FontSize=11,Opacity=.65,TextWrapping=TextWrapping.Wrap});
        var progress=new ProgressBar{IsIndeterminate=false,Value=0,Maximum=1,Height=4};body.Children.Add(progress);var detail=new StackPanel{Spacing=8};body.Children.Add(detail);
        Button? scan=null;scan=FlatButton("Analyze folder","\uEDA2",async()=>{scan!.IsEnabled=false;progress.IsIndeterminate=true;detail.Children.Clear();try{
            var result=await api.Post("devices/storage",new JsonObject{["device"]=device,["path"]=folder.Text.Trim()});
            detail.Children.Add(Label(DiskBytes(Number(result["logical_bytes"])),24));
            detail.Children.Add(new TextBlock{Text=$"{result["files"]} files · {result["duration_ms"]} ms · {(result["complete"]?.GetValue<bool>()==true?"Complete snapshot":"Partial snapshot")}\n{result["skipped"]} entries skipped · logical bytes; hard links counted per path",FontSize=11,Opacity=.65,TextWrapping=TextWrapping.Wrap});
            var largest=result["largest"]?.AsArray()??[];var maximum=Math.Max(1,largest.Select(r=>Number(r?["logical"])).DefaultIfEmpty(0).Max());
            foreach(var file in largest.Take(8)){if(file==null)continue;var entry=new StackPanel{Spacing=4};entry.Children.Add(new TextBlock{Text=file["name"]+" · "+DiskBytes(Number(file["logical"])),FontSize=11,TextTrimming=TextTrimming.CharacterEllipsis});entry.Children.Add(new ProgressBar{Value=Number(file["logical"]),Maximum=maximum,Height=4,Foreground=accent});detail.Children.Add(entry);}progress.Value=1;
            detail.Children.Add(new TextBlock{Text="Drive health · Unavailable. Live collection remains blocked.",FontSize=10,Opacity=.6,TextWrapping=TextWrapping.Wrap});
        }catch(Exception e){detail.Children.Add(new TextBlock{Text=e.Message,TextWrapping=TextWrapping.Wrap,FontSize=11});progress.Value=0;}finally{progress.IsIndeterminate=false;scan.IsEnabled=true;}});body.Children.Add(scan);Popover("Remote storage",new ScrollViewer{Content=body,MaxHeight=490},width:400);return Task.CompletedTask;
    }
}
