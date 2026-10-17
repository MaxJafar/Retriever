using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using System.Text.Json.Nodes;

namespace Retriever.Native;
public sealed partial class MainWindow {
    async Task MaybeShowExplorerWelcome(){
#if !RETRIEVER_TESTING
        if(!quick&&config?["configured"]?.GetValue<bool>()!=true&&!File.Exists(Path.Combine(api.StatePath,"onboarding.json")))await ExplorerWelcome();
#else
        await Task.CompletedTask;
#endif
    }
    Task ExplorerWelcome(){
        var body=new StackPanel{Spacing=12};
        body.Children.Add(new TextBlock{Text="Your folders. Your search.",FontSize=18,FontWeight=Microsoft.UI.Text.FontWeights.SemiBold});
        body.Children.Add(new TextBlock{Text="Expand a drive in the sidebar to browse immediately. Smart search builds a private index only for folders or drives you choose.",TextWrapping=TextWrapping.Wrap,Opacity=.75});
        body.Children.Add(FlatButton("Set up smart search","\uE721",async()=>{await ClosePopover();await IncludeFolder();}));
        body.Children.Add(FlatButton("Continue browsing","\uE8B7",()=>{File.WriteAllText(Path.Combine(api.StatePath,"onboarding.json"),"{\"version\":1,\"choice\":\"browse\"}");activePopover?.Hide();return Task.CompletedTask;}));
        body.Children.Add(FlatButton("Folder integration","\uE7F4",async()=>{await ClosePopover();await FolderIntegrationPopover();}));
        body.Children.Add(new TextBlock{Text="You can revisit setup in Disk Assistant. Sources stay read-only during crawling; generated data stays in Retriever's private archive.",FontSize=11,Opacity=.6,TextWrapping=TextWrapping.Wrap});
        Popover("Welcome to Retriever",body,width:340);return Task.CompletedTask;
    }
    async Task ReviewCrawlScope(string path){
        await RefreshConfig();var local=config!["devices"]!.AsArray().First(d=>d?["transport"]?.ToString()=="local")!;var id=local["id"]!.ToString();
        var plan=await api.Post("crawl/preview",new JsonObject{["device"]=id,["root"]=path});
        var snapshot=config.DeepClone();var expected=plan["revision"]!.ToString();
        var body=new StackPanel{Spacing=10};body.Children.Add(new TextBlock{Text=plan["whole_volume"]?.GetValue<bool>()==true?"Review this drive":"Review this folder",FontSize=17,FontWeight=Microsoft.UI.Text.FontWeights.SemiBold});
        body.Children.Add(new TextBlock{Text=path,TextWrapping=TextWrapping.Wrap,IsTextSelectionEnabled=true});
        body.Children.Add(Label("Always excluded",11));foreach(var rule in plan["protections"]?.AsArray()??[])body.Children.Add(new TextBlock{Text="· "+rule,FontSize=11,Opacity=.7,TextWrapping=TextWrapping.Wrap});
        body.Children.Add(new TextBlock{Text="The first crawl builds local word search. Model analysis and recurring schedules are separate choices. Large scopes may finish with partial coverage; progress reports show what was skipped.",TextWrapping=TextWrapping.Wrap,FontSize=12});
        body.Children.Add(new TextBlock{Text="Archive · "+plan["archive_path"],FontSize=10,Opacity=.55,TextWrapping=TextWrapping.Wrap});
        var error=new TextBlock{TextWrapping=TextWrapping.Wrap,Foreground=Brush(0xe5a088),FontSize=11};body.Children.Add(error);
        var saving=false;
        async Task Save(bool start){if(saving)return;saving=true;try{var cfg=snapshot!.DeepClone();var device=cfg["devices"]!.AsArray().First(d=>d?["id"]?.ToString()==id)!;var roots=device["roots"]!.AsArray();if(!roots.Any(p=>p?.ToString()==path))roots.Add(path);cfg["configured"]=true;await api.Post("config",cfg,expected);await RefreshConfig();File.WriteAllText(Path.Combine(api.StatePath,"onboarding.json"),"{\"version\":1,\"choice\":\"search\"}");activePopover?.Hide();if(start){await api.Post("crawl/run",new JsonObject{["device"]=id,["root"]=path,["index_only"]=true});await Crawling();}else await EngineFoldersPopover();}catch(Exception e){error.Text=e.Message;}finally{saving=false;}}
        body.Children.Add(FlatButton("Save & start local crawl","\uE768",()=>Save(true)));body.Children.Add(FlatButton("Save for later","\uE73E",()=>Save(false)));
        Popover("Smart search setup",new ScrollViewer{Content=body,MaxHeight=520},width:340);
    }
}
