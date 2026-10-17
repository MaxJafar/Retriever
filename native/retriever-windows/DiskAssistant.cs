using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using System.Text.Json.Nodes;

namespace Retriever.Native;

public sealed partial class MainWindow {
    ScrollViewer? assistantPanel;
    readonly Dictionary<string,JsonArray> explorerLocations=new();
    async Task<JsonArray> DeviceLocations(string id){
#if RETRIEVER_TESTING
        if(Environment.GetEnvironmentVariable("RETRIEVER_PROFILE")?.StartsWith("workspace-smoke-health-offline-")==true&&Environment.GetEnvironmentVariable("RETRIEVER_FIXTURE_SHOW_FILESYSTEM")!="1")return new JsonArray((config?["devices"]?.AsArray().FirstOrDefault(d=>d?["id"]?.ToString()==id)?["roots"]?.AsArray()??[]).Select(p=>(JsonNode)new JsonObject{["path"]=p!.ToString(),["name"]=Path.GetFileName(p.ToString()),["kind"]="fixture"}).ToArray());
#endif
        return (await api.Post("explorer/locations",new JsonObject{["device"]=id}))["locations"]?.AsArray()??[];
    }
    async Task BrowseDevice(string id){
        var locations=await DeviceLocations(id);explorerLocations[id]=locations.DeepClone().AsArray();
        var body=new StackPanel{Spacing=6};foreach(var entry in locations){if(entry==null)continue;var path=entry["path"]!.ToString();body.Children.Add(FlatButton(entry["name"]?.ToString()??path,"\uEDA2",async()=>{activePopover?.Hide();await Browse(id,path);}));}
        body.Children.Add(new TextBlock{Text="Browsing does not add folders to AI search. Choose crawl folders in Disk Assistant.",FontSize=10,Opacity=.6,TextWrapping=TextWrapping.Wrap});Popover(DeviceName(id),body,width:320);
    }
    async Task OpenDiskAssistant(){
        var state=EmptyTabState();state["route"]="assistant";state["title"]="Disk Assistant";state["nav"]="assistant";await NavigateDestination(state);
    }
    async Task ShowDiskAssistant(){
        var generation=querySequence;var data=await api.Get("assistant");if(route!="assistant"||generation!=querySequence)return;StopDiskPolling();StopHealthPolling();crawlTimer?.Stop();typing?.Cancel();
        route="assistant";currentTitle="Disk Assistant";SelectNavigation("assistant");
        var body=new StackPanel{Spacing=18,Padding=new Thickness(24),MaxWidth=1080,HorizontalAlignment=HorizontalAlignment.Stretch};
        var heading=new StackPanel{Spacing=6};heading.Children.Add(new TextBlock{Text="Disk Assistant",FontSize=26,FontWeight=Microsoft.UI.Text.FontWeights.SemiBold});heading.Children.Add(new TextBlock{Text="A clear view of what Retriever has learned, what is running, and what stays on your device.",Opacity=.65,TextWrapping=TextWrapping.Wrap});body.Children.Add(heading);
        var metrics=new Grid{ColumnSpacing=12};for(int i=0;i<3;i++)metrics.ColumnDefinitions.Add(new());
        var cards=new[]{InsightCard("Indexed files",Metric(Number(data["files"]).ToString("N0"))),InsightCard("Searchable passages",Metric(Number(data["passages"]).ToString("N0"))),InsightCard("Reusable vectors",Metric(Number(data["embedding_cache_entries"]).ToString("N0")))};
        for(int i=0;i<cards.Length;i++){Grid.SetColumn(cards[i],i);metrics.Children.Add(cards[i]);}body.Children.Add(metrics);
        var flow=new StackPanel{Spacing=12};foreach(var (label,count) in new[]{("Crawl · selected folders",(long)(data["folders"]?.AsArray().Count??0)),("Extract · passages",Number(data["passages"])),("Reuse · cached vectors",Number(data["embedding_cache_entries"])),("Retrieve · vector records",Number(data["vectors"]))}){
            var row=new Grid{ColumnSpacing=14};row.ColumnDefinitions.Add(new(){Width=new GridLength(210)});row.ColumnDefinitions.Add(new(){Width=new GridLength(1,GridUnitType.Star)});row.ColumnDefinitions.Add(new(){Width=new GridLength(70)});row.Children.Add(Label(label));var bar=new ProgressBar{Value=count,Maximum=Math.Max(1,new[]{Number(data["files"]),Number(data["passages"]),Number(data["vectors"])}.Max()),Foreground=accent,Height=5,VerticalAlignment=VerticalAlignment.Center};Grid.SetColumn(bar,1);row.Children.Add(bar);var amount=Label(count.ToString("N0"));amount.HorizontalAlignment=HorizontalAlignment.Right;Grid.SetColumn(amount,2);row.Children.Add(amount);flow.Children.Add(row);
        }body.Children.Add(InsightCard("Your retrieval archive",flow));
        var models=new StackPanel{Spacing=9};foreach(var model in data["models"]?.AsArray()??[]){if(model==null)continue;var role=model["role"]!.ToString();var label=role=="relevance"?"Laya · relevance":"EmbeddingGemma · meaning";var row=new Grid();row.ColumnDefinitions.Add(new(){Width=new GridLength(1,GridUnitType.Star)});row.ColumnDefinitions.Add(new(){Width=GridLength.Auto});var lines=new StackPanel{Spacing=4};lines.Children.Add(Label(label,13));lines.Children.Add(new TextBlock{Text=(model["model"]?.ToString()??"")+" · "+model["state"],FontSize=11,Opacity=.6,TextWrapping=TextWrapping.Wrap});row.Children.Add(lines);var badge=Label(model["verified"]?.ToString()=="true"?"Verified configuration":"Not verified",11);badge.Opacity=.65;Grid.SetColumn(badge,1);row.Children.Add(badge);models.Children.Add(row);}
        models.Children.Add(new TextBlock{Text=$"Coordinator uptime: {TimeSpan.FromSeconds(Number(data["uptime_seconds"])):g}. Models load on demand inside the engine.",FontSize=10,Opacity=.55,TextWrapping=TextWrapping.Wrap});body.Children.Add(InsightCard("Model ensemble",models));
        var actions=new StackPanel{Orientation=Orientation.Horizontal,Spacing=8};actions.Children.Add(FlatButton("Crawl folders & schedules","\uE8B7",EngineFoldersPopover));actions.Children.Add(FlatButton("Local search models","\uE713",Settings));actions.Children.Add(FlatButton("Refresh","\uE72C",ShowDiskAssistant));body.Children.Add(actions);
        body.Children.Add(new TextBlock{Text="RAG archive · "+data["archive_path"]+"\nDerived catalogue metadata is refreshed after each crawl. Source text and vectors stay in the private search index.",FontSize=11,Opacity=.6,TextWrapping=TextWrapping.Wrap,IsTextSelectionEnabled=true});
        page.Children.Clear();page.Children.Add(explorerChrome);foreach(var child in explorerChrome.Children.Skip(1))child.Visibility=Visibility.Collapsed;explorerViews.Visibility=Visibility.Collapsed;assistantPanel=new ScrollViewer{Content=body};Grid.SetRow(assistantPanel,1);Grid.SetRowSpan(assistantPanel,2);page.Children.Add(assistantPanel);PersistTabs();
    }
    async Task EngineFoldersPopover(){
        var data=await api.Get("crawl");var body=new StackPanel{Spacing=10};body.Children.Add(FlatButton("Explorer & search setup","\uE80F",async()=>{await ClosePopover();await ExplorerWelcome();}));
        foreach(var folder in data["folders"]?.AsArray()??[]){if(folder==null)continue;var current=folder.DeepClone();var label=new TextBlock{Text=folder["root"]?.ToString(),TextWrapping=TextWrapping.Wrap,FontSize=12};body.Children.Add(label);body.Children.Add(Label($"{folder["interval_minutes"]} min · {folder["state"]}",10));body.Children.Add(FlatButton("Indexing schedule","\uE787",async()=>{await ClosePopover();await EditCrawl(current.AsObject());}));}
        body.Children.Add(FlatButton("Choose a crawl folder","\uE710",async()=>{await ClosePopover();await IncludeFolder();}));body.Children.Add(FlatButton("Full crawl controls","\uE9E9",async()=>{activePopover?.Hide();await Crawling();}));
        Popover("AI search · crawl folders",new ScrollViewer{Content=body,MaxHeight=440},width:380);
    }
}
