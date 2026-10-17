using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using System.Text.Json.Nodes;

namespace Retriever.Native;

public sealed partial class MainWindow {
    ScrollViewer? burnerPanel;
    DispatcherTimer? burnerTimer;
    JsonNode? burnerData;
    readonly TextBlock burnerSource=new(){Text="Choose an image",FontSize=15,TextWrapping=TextWrapping.Wrap};
    readonly TextBlock burnerSourceDetail=new(){FontSize=11,Opacity=.65,TextWrapping=TextWrapping.Wrap};
    readonly TextBlock burnerState=new(){FontSize=15};
    readonly TextBlock burnerProgressDetail=new(){FontSize=11,Opacity=.65,TextWrapping=TextWrapping.Wrap};
    readonly TextBlock burnerNotice=new(){FontSize=12,TextWrapping=TextWrapping.Wrap};
    readonly TextBlock burnerTargetDetail=new(){FontSize=11,Opacity=.65,TextWrapping=TextWrapping.Wrap};
    readonly ProgressBar burnerProgress=new(){Minimum=0,Maximum=100,Height=5};
    readonly TextBox burnerChecksum=new(){PlaceholderText="Optional publisher SHA-256",MaxLength=64,FontSize=12};
    readonly StackPanel burnerHistory=new(){Spacing=6};
    readonly StackPanel burnerSteps=new(){Orientation=Orientation.Horizontal,Spacing=22};
    readonly ComboBox burnerTarget=new(){HorizontalAlignment=HorizontalAlignment.Stretch,MinWidth=150};
    Button? burnerChoose, burnerSample, burnerStart, burnerCancel;
    bool burnerRefreshing, burnerChoosing;
    string? burnerActionError;
    string burnerHistoryKey="";
    static bool BurnerRunning(JsonNode? job)=>job?["state"]?.ToString() is "inspecting" or "preflight" or "writing" or "verifying";
    static string BurnerStage(string? stage)=>stage switch {"inspecting"=>"Inspecting image","preflight"=>"Checking source again","writing"=>"Writing file target","verifying"=>"Reading back every byte","ready"=>"Image inspected","complete"=>"Rehearsal verified","cancelled"=>"Cancelled","interrupted"=>"Interrupted","failed"=>"Task failed",_=>"Ready when you are"};
    void StopBurnerPolling(){burnerTimer?.Stop();burnerTimer=null;}
    async Task OpenDiskBurner(){
        var state=EmptyTabState();state["route"]="burner";state["title"]="Disk Burner";state["nav"]="burner";await NavigateDestination(state);
    }
    async Task ShowDiskBurner(){
        StopDiskPolling();StopHealthPolling();StopBurnerPolling();crawlTimer?.Stop();typing?.Cancel();
        route="burner";currentTitle="Disk Burner";SelectNavigation("burner");
        if(burnerPanel==null)BuildBurnerPanel();
        foreach(var child in page.Children.Where(c=>c!=explorerChrome).ToArray())page.Children.Remove(child);
        if(!page.Children.Contains(explorerChrome))page.Children.Add(explorerChrome);
        foreach(var child in explorerChrome.Children.Skip(1))child.Visibility=Visibility.Collapsed;explorerViews.Visibility=Visibility.Collapsed;
        Grid.SetRow(burnerPanel!,1);Grid.SetRowSpan(burnerPanel!,2);page.Children.Add(burnerPanel!);PersistTabs();
        await RefreshBurner();
        if(route!="burner")return;
        burnerTimer=new DispatcherTimer{Interval=TimeSpan.FromMilliseconds(250)};
        burnerTimer.Tick+=async(_,_)=>{if(route!="burner"){StopBurnerPolling();return;}if(burnerRefreshing)return;burnerRefreshing=true;try{await RefreshBurner();}catch(Exception ex){burnerNotice.Text=ex.Message;StopBurnerPolling();}finally{burnerRefreshing=false;}};burnerTimer.Start();
    }
    void BuildBurnerPanel(){
        var body=new StackPanel{Spacing=18,Padding=new Thickness(24),MaxWidth=1080,HorizontalAlignment=HorizontalAlignment.Stretch};
        var heading=new Grid();heading.ColumnDefinitions.Add(new(){Width=new GridLength(1,GridUnitType.Star)});heading.ColumnDefinitions.Add(new(){Width=GridLength.Auto});
        var title=new StackPanel{Spacing=5};title.Children.Add(new TextBlock{Text="Disk Burner",FontSize=26,FontWeight=Microsoft.UI.Text.FontWeights.SemiBold});title.Children.Add(new TextBlock{Text="Inspect an image. Rehearse the write. Verify every byte.",FontSize=12,Opacity=.65});heading.Children.Add(title);
        var badge=new Border{Background=Brush(0x443d29),CornerRadius=new CornerRadius(5),Padding=new Thickness(10,5,10,5),VerticalAlignment=VerticalAlignment.Top,Child=new TextBlock{Text="FILE SANDBOX",FontSize=10,Foreground=accent}};Grid.SetColumn(badge,1);heading.Children.Add(badge);body.Children.Add(heading);
        body.Children.Add(new TextBlock{Text="Physical disk writing is disabled in this candidate. Rehearsals use a temporary file, which is removed after the test.",FontSize=12,Opacity=.7,TextWrapping=TextWrapping.Wrap});
        foreach(var label in new[]{"01  Inspect","02  Check","03  Write","04  Verify"})burnerSteps.Children.Add(new TextBlock{Text=label,FontSize=12,Opacity=.45});body.Children.Add(burnerSteps);
        var choices=new Grid{ColumnSpacing=14,RowSpacing=14};choices.ColumnDefinitions.Add(new());choices.ColumnDefinitions.Add(new());choices.RowDefinitions.Add(new());choices.RowDefinitions.Add(new());
        var imageBody=new StackPanel{Spacing=10};imageBody.Children.Add(burnerSource);imageBody.Children.Add(burnerSourceDetail);
        var imageActions=new StackPanel{Orientation=Orientation.Horizontal,Spacing=5};burnerChoose=FlatButton("Choose image","\uE8B7",PickBurnerImage);imageActions.Children.Add(burnerChoose);
        imageActions.Children.Add(FlatButton("Checksum","\uE9F9",()=>{var panel=new StackPanel{Spacing=8};panel.Children.Add(new TextBlock{Text="Paste the publisher’s SHA-256 before choosing an image. A matching hash confirms integrity, not publisher identity.",FontSize=11,Opacity=.7,TextWrapping=TextWrapping.Wrap});if(burnerChecksum.Parent is Panel previous)previous.Children.Remove(burnerChecksum);panel.Children.Add(burnerChecksum);Popover("Verify source checksum",panel,width:320);return Task.CompletedTask;}));imageBody.Children.Add(imageActions);
        burnerSample=FlatButton("Try a sample image","\uE768",()=>SelectBurnerImage(async()=>{burnerChecksum.Text="";var sample=await api.Post("burner/sample");return sample["path"]!.ToString();}));imageBody.Children.Add(burnerSample);
        var imageCard=InsightCard("01 · Source image",imageBody);choices.Children.Add(imageCard);
        var targetBody=new StackPanel{Spacing=10};targetBody.Children.Add(new TextBlock{Text="Disposable file target",FontSize=15});targetBody.Children.Add(burnerTarget);targetBody.Children.Add(burnerTargetDetail);
        targetBody.Children.Add(new TextBlock{Text="Simulated capacity · allocated only as bytes are written. No USB or internal drives are opened.",FontSize=11,Opacity=.65,TextWrapping=TextWrapping.Wrap});
        var targetCard=InsightCard("02 · Destination",targetBody);Grid.SetColumn(targetCard,1);choices.Children.Add(targetCard);
        choices.SizeChanged+=(_,_)=>{bool narrow=choices.ActualWidth<620;Grid.SetColumn(targetCard,narrow?0:1);Grid.SetRow(targetCard,narrow?1:0);Grid.SetColumnSpan(imageCard,narrow?2:1);Grid.SetColumnSpan(targetCard,narrow?2:1);};body.Children.Add(choices);
        burnerTarget.SelectionChanged+=(_,_)=>UpdateBurner();
        var flow=new StackPanel{Spacing=12};flow.Children.Add(burnerState);burnerProgress.Foreground=accent;AutomationProperties.SetName(burnerProgress,"Disk Burner current phase progress");flow.Children.Add(burnerProgress);flow.Children.Add(burnerProgressDetail);flow.Children.Add(burnerNotice);
        var actions=new StackPanel{Orientation=Orientation.Horizontal,Spacing=8};burnerStart=FlatButton("Rehearse write & verify","\uE958",ReviewBurner);actions.Children.Add(burnerStart);
        burnerCancel=FlatButton("Cancel","\uE711",async()=>{var id=burnerData?["current"]?["id"]?.ToString();if(id!=null){await api.Post("burner/cancel",new JsonObject{["id"]=id});burnerNotice.Text="Cancelling…";}});actions.Children.Add(burnerCancel);flow.Children.Add(actions);
        body.Children.Add(InsightCard("Write verification",flow));body.Children.Add(InsightCard("Recent activity",burnerHistory));
        body.Children.Add(new TextBlock{Text="Raw IMG / BIN / RAW and partitioned ISO images can be rehearsed. Windows installer preparation, compressed-image conversion and physical USB writing are not available yet. A verified copy does not prove bootability.",FontSize=10,Opacity=.55,TextWrapping=TextWrapping.Wrap});
        burnerPanel=new ScrollViewer{Content=body,HorizontalScrollBarVisibility=ScrollBarVisibility.Disabled};
    }
    async Task RefreshBurner(){var data=await api.Get("burner");if(route!="burner")return;burnerData=data;
        if(burnerTarget.Items.Count==0){foreach(var target in data["targets"]?.AsArray()??[]){if(target!=null)burnerTarget.Items.Add(new ComboBoxItem{Content=target["name"]!.ToString(),Tag=target["id"]!.ToString()});}burnerTarget.SelectedIndex=0;}
        UpdateBurner();
    }
    void UpdateBurner(){
        if(burnerData==null)return;var job=burnerData["current"];var image=burnerData["inspection"]?["image"];bool busy=BurnerRunning(job);
        if(route=="burner")message.Text="Disk Burner · "+(burnerChoosing?"Choosing an image":burnerActionError!=null?"Image selection needs attention":BurnerStage(job?["state"]?.ToString()));
        burnerSource.Text=image?["name"]?.ToString()??(job?["state"]?.ToString()=="inspecting"?"Inspecting selected image…":"Choose an image");
        burnerSourceDetail.Text=image==null?"Local image files · SHA-256 inspection":$"{DiskBytes(Number(image["bytes"]))} · {image["format"]}\n{image["partition_style"]} · Bootability not verified";
        var targetId=(burnerTarget.SelectedItem as ComboBoxItem)?.Tag?.ToString();var target=burnerData["targets"]?.AsArray().FirstOrDefault(v=>v?["id"]?.ToString()==targetId);long capacity=Number(target?["capacity"]);bool fits=image!=null&&Number(image["bytes"])<=capacity;
        burnerTargetDetail.Text=image==null?"Select an image to check its size.":fits?$"{DiskBytes(Number(image["bytes"]))} of {DiskBytes(capacity)} · Fits":$"Image exceeds {DiskBytes(capacity)} target capacity.";
        burnerState.Text=BurnerStage(job?["state"]?.ToString());burnerProgress.IsIndeterminate=busy&&Number(job?["bytes_total"])==0;burnerProgress.Value=Number(job?["bytes_total"])>0?Math.Clamp(Number(job?["bytes_done"])*100d/Number(job?["bytes_total"]),0,100):0;
        burnerProgressDetail.Text=job==null?"No image selected yet.":$"{DiskBytes(Number(job["bytes_done"]))} / {DiskBytes(Number(job["bytes_total"]))}"+(busy?$" · {DiskBytes((long)(job["bytes_per_second"]?.GetValue<double>()??0))}/s":"")+(job["state"]?.ToString()=="complete"?" · SHA-256 match · Temporary target removed":"");
        burnerNotice.Text=burnerActionError??job?["error"]?.ToString()??(image?["notes"] is JsonArray notes?string.Join("\n",notes.Select(n=>n?.ToString())):"");
        if(burnerChoose!=null)burnerChoose.IsEnabled=!busy&&!burnerChoosing;if(burnerSample!=null)burnerSample.IsEnabled=!busy&&!burnerChoosing;burnerTarget.IsEnabled=!busy&&!burnerChoosing;
        if(burnerStart!=null)burnerStart.IsEnabled=!busy&&!burnerChoosing&&burnerActionError==null&&fits&&image?["raw_copy"]?.GetValue<bool>()==true;if(burnerCancel!=null)burnerCancel.IsEnabled=busy;
        int step=job?["state"]?.ToString() switch {"inspecting"=>0,"preflight"=>1,"writing"=>2,"verifying" or "complete"=>3,_=>-1};
        for(int i=0;i<burnerSteps.Children.Count;i++)burnerSteps.Children[i].Opacity=i==step?1:.45;
        var history=burnerData["history"]?.AsArray()??[];var key=string.Join("|",history.Take(6).Select(h=>h?["id"]+":"+h?["state"]));
        if(key==burnerHistoryKey&&burnerHistory.Children.Count>0)return;burnerHistoryKey=key;burnerHistory.Children.Clear();
        foreach(var row in history.Take(6)){if(row==null)continue;var saved=row.DeepClone();burnerHistory.Children.Add(FlatButton($"{BurnerStage(row["state"]?.ToString())} · {row["image"]?["name"]?.ToString()??"Image inspection"}",row["state"]?.ToString()=="complete"?"\uE73E":"\uE9F9",()=>BurnerReceipt(saved)));}
        if(history.Count==0)burnerHistory.Children.Add(new TextBlock{Text="Inspection and verification receipts appear here.",FontSize=11,Opacity=.55});
    }
    Task PickBurnerImage(){
        if(api.Mode=="client"){burnerActionError="Choose images on the computer running this coordinator. Remote file picking is not available yet.";UpdateBurner();return Task.CompletedTask;}
        return SelectBurnerImage(async()=>{
            await Task.Yield();var previousDialog=dialogVisible;dialogVisible=true;
            try{return await PickPath("Choose a disk image",System.IO.Path.GetDirectoryName(burnerData?["inspection"]?["image"]?["path"]?.ToString()??""),[".iso",".img",".bin",".raw",".zip",".gz",".xz",".dmg",".vhd",".vhdx"]);}
            finally{dialogVisible=previousDialog;}
        });
    }
    async Task SelectBurnerImage(Func<Task<string?>> select){
        if(burnerChoosing||BurnerRunning(burnerData?["current"]))return;
        burnerChoosing=true;UpdateBurner();
        try{var path=await select();if(path!=null)await InspectBurnerImage(path);}
        catch(OperationCanceledException){ /* Closing the picker keeps the previous image and receipt. */ }
        catch(Exception ex){burnerActionError="Could not select this image. "+ex.Message;}
        finally{burnerChoosing=false;UpdateBurner();}
    }
    async Task InspectBurnerImage(string path){await api.Post("burner/inspect",new JsonObject{["path"]=path,["sha256"]=burnerChecksum.Text.Trim()});burnerActionError=null;await RefreshBurner();}
    Task ReviewBurner(){
        if(burnerChoosing||burnerActionError!=null||BurnerRunning(burnerData?["current"]))return Task.CompletedTask;
        var inspection=burnerData?["inspection"]?.DeepClone();var target=(burnerTarget.SelectedItem as ComboBoxItem)?.Tag?.ToString();if(inspection==null||target==null)return Task.CompletedTask;
        var body=new StackPanel{Spacing=12};body.Children.Add(new TextBlock{Text=$"{inspection["image"]?["name"]}\n{DiskBytes(Number(inspection["image"]?["bytes"]))} → {((ComboBoxItem)burnerTarget.SelectedItem).Content}",FontSize=12,TextWrapping=TextWrapping.Wrap});
        body.Children.Add(new TextBlock{Text="Retriever will recheck the source, write a new temporary file, read it back and compare SHA-256. The temporary file is removed afterwards.",FontSize=11,Opacity=.7,TextWrapping=TextWrapping.Wrap});
        body.Children.Add(FlatButton("Start rehearsal","\uE768",async()=>{activePopover?.Hide();await api.Post("burner/start",new JsonObject{["inspection_id"]=inspection["id"]!.ToString(),["target_id"]=target});await RefreshBurner();}));
        Popover("Review file rehearsal",body,burnerStart,320);return Task.CompletedTask;
    }
    Task BurnerReceipt(JsonNode job){
        var body=new StackPanel{Spacing=8};body.Children.Add(new TextBlock{Text=BurnerStage(job["state"]?.ToString()),FontSize=14});
        var text=$"{job["image"]?["name"]}\n{DiskBytes(Number(job["image"]?["bytes"]))}\nSource SHA-256\n{job["image"]?["sha256"]}\nRead-back SHA-256\n{job["verified_sha256"]?.ToString()??"Not verified"}";
        if(job["error"]!=null)text+="\n"+job["error"];text+="\nBootability: not verified. Physical writes: disabled.";
        body.Children.Add(new TextBlock{Text=text,FontSize=11,TextWrapping=TextWrapping.Wrap,IsTextSelectionEnabled=true});Popover("Verification receipt",body,width:340);return Task.CompletedTask;
    }
}
