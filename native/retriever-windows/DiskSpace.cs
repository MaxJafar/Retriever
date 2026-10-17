using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Markup;
using System.Collections.ObjectModel;
using System.Diagnostics;
using System.Text.Json.Nodes;

namespace Retriever.Native;

public sealed partial class MainWindow {
    sealed class DiskEntry(JsonNode data) {
        public JsonNode Data{get;}=data;
        public long Id=>Number(Data["id"]);
        public string Name=>Data["name"]?.ToString()??"";
        public string Path=>Data["path"]?.ToString()??"";
        public bool Folder=>Data["kind"]?.ToString()=="folder";
        public string Glyph=>Folder?"\uE8B7":"\uE8A5";
        public string Logical=>DiskBytes(Number(Data["logical"]));
        public string Allocated=>DiskBytes(Number(Data["allocated"]))+(Number(Data["allocation_unknown"])>0?" + ?":"");
        public string Change=>Data["delta"]==null?"—":(Number(Data["delta"])>0?"+":"")+DiskBytes(Number(Data["delta"]));
        public string Detail=>Path+(string.IsNullOrEmpty(Data["warning"]?.ToString())?"":"\n"+Data["warning"]);
    }
    readonly ObservableCollection<DiskEntry> diskRows=[];
    readonly ListView diskList=new(){SelectionMode=ListViewSelectionMode.Single};
    readonly Canvas diskMap=new(){Height=152,HorizontalAlignment=HorizontalAlignment.Stretch};
    readonly TextBlock diskPath=Label("Choose a folder or drive"),diskSummary=Label(""),diskNote=Label(""),diskTypes=Label(""),diskPage=Label("");
    Grid? diskPanel;
    JsonNode? diskResult;
    DispatcherQueueTimer? diskTimer;
    Button? diskScan,diskCancel,diskUp,diskNext,diskPrevious,diskReveal,diskRelated;
    ComboBox? diskMode;
    bool diskFetching,diskRendering;
    long diskGeneration,diskRequest;
    static long Number(JsonNode? value)=>long.TryParse(value?.ToString(),out var number)?number:0;
    internal static string DiskBytes(long value){var sign=value<0?"−":"";double size=Math.Abs((double)value);var units=new[]{"B","KiB","MiB","GiB","TiB"};int i=0;while(size>=1024&&i<units.Length-1){size/=1024;i++;}return sign+size.ToString(i==0?"N0":"0.0")+" "+units[i];}
    async Task OpenDiskSpace(){
        if(quick)return;
        var local=config?["devices"]?.AsArray().FirstOrDefault(d=>d?["transport"]?.ToString()=="local")?["id"]?.ToString();
        var rootPath=route=="browse"&&location["device"]?.ToString()==local?location["path"]?.ToString():null;
        await NavigateDestination(new JsonObject{["route"]="disk",["title"]="Disk Capacity",["nav"]="disk",["query"]="",["location"]=new JsonObject{["root"]=rootPath??Environment.GetFolderPath(Environment.SpecialFolder.UserProfile)}});
    }
    void StopDiskPolling(){diskTimer?.Stop();diskGeneration++;}
    void BuildDiskPanel(){
        diskPanel=new Grid{Padding=new Thickness(12,0,12,0)};
        foreach(var height in new[]{GridLength.Auto,GridLength.Auto,GridLength.Auto,new GridLength(152),GridLength.Auto,GridLength.Auto,new GridLength(1,GridUnitType.Star),GridLength.Auto})diskPanel.RowDefinitions.Add(new(){Height=height});
        void Add(FrameworkElement element,int row){Grid.SetRow(element,row);diskPanel.Children.Add(element);}
        var actions=new StackPanel{Orientation=Orientation.Horizontal,Spacing=6,Margin=new Thickness(0,5,0,5)};
        actions.Children.Add(FlatButton("Choose folder","\uED25",ChooseDiskFolder));
        diskScan=FlatButton("Scan / rescan","\uE9F9",StartDiskScan);actions.Children.Add(diskScan);
        diskCancel=FlatButton("Cancel scan","\uE711",async()=>{await api.Post("disk/cancel",new JsonObject{["id"]=location["scan"]?.DeepClone()});await RefreshDisk();});actions.Children.Add(diskCancel);
        diskUp=FlatButton("Parent folder","\uE70E",async()=>{location["node"]=Math.Max(1,Number(diskResult?["current"]?["parent"]));location["offset"]=0;await RefreshDisk();},true);actions.Children.Add(diskUp);
        actions.Children.Add(FlatButton("Scan root","\uE80F",async()=>{location["node"]=1;location["offset"]=0;await RefreshDisk();},true));Add(actions,0);
        diskPath.FontSize=11;diskPath.Margin=new Thickness(0,4,0,8);Add(diskPath,1);
        diskSummary.FontSize=14;diskSummary.Margin=new Thickness(0,0,0,10);Add(capacityOverview,2);RenderCapacityOverview();
        diskMap.SizeChanged+=(_,_)=>RenderDiskMap();Add(diskMap,3);
        diskTypes.FontSize=11;diskTypes.Opacity=.7;diskTypes.Margin=new Thickness(0,7,0,7);Add(diskTypes,4);
        var row=new Grid{ColumnSpacing=10};row.ColumnDefinitions.Add(new(){Width=new GridLength(1,GridUnitType.Star)});row.ColumnDefinitions.Add(new(){Width=GridLength.Auto});
        diskMode=new ComboBox{ItemsSource=new[]{"Folders & files","Largest files","Not modified in a year","Size changes"},SelectedIndex=0,MinWidth=190,MinHeight=28,Margin=new Thickness(0,0,0,6)};
        diskMode.SelectionChanged+=async(_,_)=>{if(diskRendering||route!="disk")return;location["mode"]=new[]{"children","largest","older","growth"}[diskMode.SelectedIndex];location["offset"]=0;await Guard(RefreshDisk);};row.Children.Add(diskMode);
        var rowActions=new StackPanel{Orientation=Orientation.Horizontal,Spacing=6};
        diskReveal=FlatButton("Open in Retriever","\uE8B7",RevealDiskEntry);diskRelated=FlatButton("Find related","\uE721",FindDiskRelated);diskReveal.IsEnabled=false;diskRelated.IsEnabled=false;rowActions.Children.Add(diskReveal);rowActions.Children.Add(diskRelated);Grid.SetColumn(rowActions,1);row.Children.Add(rowActions);Add(row,5);
        diskList.ItemsSource=diskRows;diskList.SelectionChanged+=(_,_)=>{diskRelated!.IsEnabled=diskList.SelectedItem!=null;diskReveal!.IsEnabled=diskList.SelectedItem!=null&&api.Mode!="client";};
        diskList.ItemTemplate=(DataTemplate)XamlReader.Load("<DataTemplate xmlns='http://schemas.microsoft.com/winfx/2006/xaml/presentation'><Grid Height='31' ColumnSpacing='8' ToolTipService.ToolTip='{Binding Detail}'><Grid.ColumnDefinitions><ColumnDefinition Width='22'/><ColumnDefinition Width='*'/><ColumnDefinition Width='90'/><ColumnDefinition Width='100'/><ColumnDefinition Width='90'/></Grid.ColumnDefinitions><FontIcon Glyph='{Binding Glyph}' Foreground='#E5BD4C' FontSize='15'/><TextBlock Grid.Column='1' Text='{Binding Name}' TextTrimming='CharacterEllipsis' VerticalAlignment='Center'/><TextBlock Grid.Column='2' Text='{Binding Logical}' HorizontalAlignment='Right' VerticalAlignment='Center'/><TextBlock Grid.Column='3' Text='{Binding Allocated}' HorizontalAlignment='Right' VerticalAlignment='Center' Opacity='.7'/><TextBlock Grid.Column='4' Text='{Binding Change}' HorizontalAlignment='Right' VerticalAlignment='Center' Opacity='.7'/></Grid></DataTemplate>");
        var style=new Style(typeof(ListViewItem));style.Setters.Add(new Setter(Control.HorizontalContentAlignmentProperty,HorizontalAlignment.Stretch));style.Setters.Add(new Setter(Control.PaddingProperty,new Thickness(4,0,4,0)));style.Setters.Add(new Setter(FrameworkElement.MinHeightProperty,31d));style.Setters.Add(new Setter(Control.FontSizeProperty,12d));diskList.ItemContainerStyle=style;
        diskList.DoubleTapped+=async(_,_)=>await Guard(EnterDiskFolder);
        diskList.KeyDown+=async(_,e)=>{if(e.Key==Windows.System.VirtualKey.Enter){e.Handled=true;await Guard(EnterDiskFolder);}};
        var listArea=new Grid();listArea.RowDefinitions.Add(new(){Height=GridLength.Auto});listArea.RowDefinitions.Add(new(){Height=new GridLength(1,GridUnitType.Star)});
        var headings=new Grid{ColumnSpacing=8,Margin=new Thickness(4,0,4,0)};foreach(var width in new[]{new GridLength(1,GridUnitType.Star),new GridLength(90),new GridLength(100),new GridLength(90)})headings.ColumnDefinitions.Add(new(){Width=width});
        int c=0;foreach(var title in new[]{"Name","Logical size","Allocated","Change"}){var label=Label(title,11);label.Height=28;if(c>0)label.HorizontalAlignment=HorizontalAlignment.Right;Grid.SetColumn(label,c++);headings.Children.Add(label);}listArea.Children.Add(headings);Grid.SetRow(diskList,1);listArea.Children.Add(diskList);Add(listArea,6);
        var footer=new StackPanel{Spacing=4,Margin=new Thickness(0,6,0,6)};var pages=new StackPanel{Orientation=Orientation.Horizontal,Spacing=10};pages.Children.Add(diskPage);
        diskPrevious=FlatButton("Previous page","",async()=>{location["offset"]=Math.Max(0,Number(location["offset"])-200);await RefreshDisk();});pages.Children.Add(diskPrevious);
        diskNext=FlatButton("Next page","",async()=>{location["offset"]=Number(diskResult?["next_offset"]);await RefreshDisk();});pages.Children.Add(diskNext);footer.Children.Add(pages);
        diskNote.FontSize=10;diskNote.Opacity=.7;diskNote.TextWrapping=TextWrapping.Wrap;diskNote.MaxLines=3;footer.Children.Add(diskNote);Add(footer,7);
    }
    async Task ShowDiskSpace(){
        StopDiskPolling();StopHealthPolling();crawlTimer?.Stop();Interlocked.Increment(ref querySequence);rows.Clear();
        if(diskPanel==null)BuildDiskPanel();
        if(!page.Children.Contains(explorerChrome)){page.Children.Clear();page.Children.Add(explorerChrome);}
        foreach(var child in explorerChrome.Children.Skip(1))child.Visibility=Visibility.Collapsed;
        explorerViews.Visibility=Visibility.Collapsed;
        foreach(var child in page.Children.ToArray())if(child!=explorerChrome&&child!=diskPanel)page.Children.Remove(child);
        if(!page.Children.Contains(diskPanel!)){Grid.SetRow(diskPanel!,1);Grid.SetRowSpan(diskPanel!,2);page.Children.Add(diskPanel!);}
        currentTitle="Disk Capacity";SelectNavigation("disk");diskResult=null;diskRows.Clear();RenderCapacityOverview();RenderDiskMap();diskPath.Text=location["root"]?.ToString()??"Choose a folder";diskNote.Text="Choose a folder or drive to measure. Scans keep your files untouched.";
        var generation=diskGeneration;
        if(location["scan"]==null){var status=await api.Get("disk");if(generation!=diskGeneration||route!="disk")return;var latest=status["scans"]?.AsArray().FirstOrDefault();if(latest!=null){location["scan"]=latest["id"]?.DeepClone();location["root"]=latest["root"]?.DeepClone();}}
        await RefreshDisk();
        if(generation!=diskGeneration||route!="disk")return;
        diskTimer??=DispatcherQueue.CreateTimer();diskTimer.Interval=TimeSpan.FromSeconds(1);
        diskTimer.Tick-=DiskTick;diskTimer.Tick+=DiskTick;diskTimer.Start();PersistTabs();
    }
    async void DiskTick(DispatcherQueueTimer sender,object args){if(!diskFetching&&route=="disk"&&diskResult?["scan"]?["state"]?.ToString()=="scanning")await Guard(RefreshDisk);}
    async Task ChooseDiskFolder(){
        if(api.Mode=="client")throw new InvalidOperationException("Analyze disks from the desktop running the coordinator.");
        var folder=await PickPath("Choose a folder to measure",location["root"]?.ToString());if(folder==null)return;
        location["root"]=folder;await StartDiskScan();
    }
    async Task StartDiskScan(){
        if(api.Mode=="client")throw new InvalidOperationException("Analyze disks from the desktop running the coordinator.");
        var generation=diskGeneration;var response=await api.Post("disk/scan",new JsonObject{["path"]=location["root"]?.DeepClone()});if(generation!=diskGeneration||route!="disk")return;
        location["scan"]=response["id"]?.DeepClone();location["node"]=1;location["offset"]=0;await RefreshDisk();
    }
    async Task RefreshDisk(){
        if(route!="disk")return;diskFetching=true;var request=++diskRequest;var generation=diskGeneration;var captured=location.ToJsonString();
        try{
            if(location["scan"]!=null){var reply=await api.Post("disk/view",new JsonObject{["id"]=location["scan"]?.DeepClone(),["node"]=location["node"]?.DeepClone()??JsonValue.Create(1),["mode"]=location["mode"]?.DeepClone()??JsonValue.Create("children"),["offset"]=location["offset"]?.DeepClone()??JsonValue.Create(0)});
                if(request!=diskRequest||generation!=diskGeneration||route!="disk")return;if(captured!=location.ToJsonString()){diskFetching=false;await RefreshDisk();return;}diskResult=reply;}
            diskRendering=true;var mode=location["mode"]?.ToString()??"children";diskMode!.SelectedIndex=Math.Max(0,Array.IndexOf(new[]{"children","largest","older","growth"},mode));
            var scan=diskResult?["scan"];var summary=scan?["summary"];var running=scan?["state"]?.ToString()=="scanning";var totals=running?summary?["totals"]:diskResult?["current"];
            diskPath.Text=(diskResult?["fixture_only"]?.ToString()=="true"?"SYNTHETIC PREVIEW · ":"")+(diskResult?["current"]?["path"]?.ToString()??location["root"]?.ToString()??"Choose a folder");ToolTipService.SetToolTip(diskPath,diskPath.Text);
            diskSummary.Text=scan==null?"Disk Capacity · Ready to scan":$"{DiskBytes(Number(totals?["logical"]))} logical  ·  {DiskBytes(Number(totals?["allocated"]))} allocated  ·  {Number(totals?["files"]):N0} files  ·  {scan["state"]}";
            diskScan!.IsEnabled=!running&&api.Mode!="client"&&diskResult?["fixture_only"]?.ToString()!="true";diskCancel!.IsEnabled=running;diskUp!.IsEnabled=Number(diskResult?["current"]?["parent"])>0;
            var selected=(diskList.SelectedItem as DiskEntry)?.Id;diskRows.Clear();foreach(var item in diskResult?["entries"]?.AsArray()??[])if(item!=null)diskRows.Add(new(item));diskList.SelectedItem=diskRows.FirstOrDefault(r=>r.Id==selected);
            diskTypes.Text=string.Join("   ·   ",(diskResult?["types"]?.AsArray()??[]).Take(5).Select(t=>(string.IsNullOrEmpty(t?["extension"]?.ToString())?"No extension":"."+t?["extension"])+" "+DiskBytes(Number(t?["logical"]))));
            diskPage.Text=$"{Number(diskResult?["total"]):N0} items · Largest first";diskPrevious!.IsEnabled=Number(location["offset"])>0;diskNext!.IsEnabled=diskResult?["next_offset"]!=null;
            var exclusions=Number(summary?["links_skipped"])+Number(summary?["mounts_skipped"])+Number(summary?["private_state_skipped"]);
            diskNote.Text=scan==null?"Scans measure local file metadata. Choose a folder or drive to start.":$"{(Number(summary?["elapsed_ms"])/1000.0):0.0}s · {Number(summary?["errors"]):N0} read errors · {exclusions:N0} links / mounts / private folders excluded · {Number(totals?["allocation_unknown"]):N0} unknown allocations. "+(mode=="older"?"Age does not mean unused. ":"")+(diskResult?["previous_scan"]==null?"Scan again for size changes. ":"")+"Allocated size is not a reclaimable-space estimate. "+scan["error"];
            var volume=summary?["volume"];if(volume!=null)diskNote.Text+=$" Volume: {DiskBytes(Number(volume["available"]))} available of {DiskBytes(Number(volume["total"]))}.";
            message.Text="Disk Capacity · "+(scan?["state"]?.ToString()??"Ready to scan");
            ToolTipService.SetToolTip(diskNote,summary?["error_samples"]?.ToJsonString());RenderCapacityOverview();RenderDiskMap();PersistTabs();
        } finally{diskRendering=false;if(request==diskRequest)diskFetching=false;}
    }
    void RenderDiskMap(){
        diskMap.Children.Clear();double width=diskMap.ActualWidth;if(width<=0)return;var tiles=diskResult?["treemap"]?.AsArray()??[];int index=0;
        foreach(var tile in tiles){if(tile==null)continue;var rect=tile["rect"];if(rect==null)continue;double Value(string key)=>rect[key]?.GetValue<double>()??0;
            var id=Number(tile["id"]);var folder=tile["kind"]?.ToString()=="folder";var name=tile["name"]?.ToString()??"";
            var button=FlatButton(name,"",async()=>{if(folder){location["node"]=id;location["offset"]=0;await RefreshDisk();}else{diskList.SelectedItem=diskRows.FirstOrDefault(r=>r.Id==id);}},true);
            button.Content=new TextBlock{Text=name+"\n"+DiskBytes(Number(tile["logical"])),FontSize=11,TextWrapping=TextWrapping.Wrap,TextTrimming=TextTrimming.CharacterEllipsis,MaxLines=3};button.Padding=new Thickness(7);button.Foreground=Brush(0xffffff);button.Background=Brush(new uint[]{0x6a5827,0x564c30,0x3e493d,0x464347,0x52513a}[index++%5]);button.MinWidth=0;button.MinHeight=0;button.Width=Math.Max(0,Value("width")*width-2);button.Height=Math.Max(0,Value("height")*152-2);if(button.Width<65||button.Height<30)button.Content=null;button.IsEnabled=id>0;Canvas.SetLeft(button,Value("x")*width);Canvas.SetTop(button,Value("y")*152);AutomationProperties.SetName(button,$"{name}, {DiskBytes(Number(tile["logical"]))}" );ToolTipService.SetToolTip(button,name+" · "+DiskBytes(Number(tile["logical"])));diskMap.Children.Add(button);
        }
        if(tiles.Count==0)diskMap.Children.Add(new TextBlock{Text=diskResult?["scan"]?["state"]?.ToString()=="scanning"?"Measuring folders…":"Your space map will appear here",Opacity=.5,Margin=new Thickness(16)});
    }
    async Task EnterDiskFolder(){if(diskList.SelectedItem is DiskEntry {Folder:true} entry){location["node"]=entry.Id;location["offset"]=0;await RefreshDisk();}}
    async Task RevealDiskEntry(){if(api.Mode=="client")throw new InvalidOperationException("Browse files on the computer that was scanned.");if(diskList.SelectedItem is not DiskEntry entry)return;var device=config?["devices"]?.AsArray().FirstOrDefault(d=>d?["transport"]?.ToString()=="local")?["id"]?.ToString()??throw new InvalidOperationException("This computer is not connected.");var file=entry.Data.DeepClone();file["device"]=device;await RevealInRetriever(new FileRow(file));}
    async Task FindDiskRelated(){if(diskList.SelectedItem is not DiskEntry entry)return;await NewTab(new JsonObject{["route"]="library",["title"]="Search results",["nav"]="search",["query"]=Path.GetFileNameWithoutExtension(entry.Name),["location"]=new JsonObject()});}
}
