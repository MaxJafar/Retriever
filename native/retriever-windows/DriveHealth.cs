using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Markup;
using Microsoft.UI.Xaml.Shapes;
using System.Collections.ObjectModel;
using System.Text.Json.Nodes;
using Windows.ApplicationModel.DataTransfer;

namespace Retriever.Native;

public sealed partial class MainWindow {
    sealed class HealthRow(string id,string name,string current,string worst,string threshold,string raw) {
        public string Id=>id;public string Name=>name;public string Current=>current;public string Worst=>worst;public string Threshold=>threshold;public string Raw=>raw;
    }
    Grid? healthPanel;
    readonly ObservableCollection<HealthRow> healthRows=[];
    readonly ListView healthList=new(){SelectionMode=ListViewSelectionMode.Single};
    readonly TextBlock healthNotice=Label("",12),healthTitle=Label("Disk Health",20),healthSummary=Label("Check your HDDs and SSDs",14),healthInfo=Label("",11),healthNote=Label("",11),healthChartLabel=Label("",11);
    readonly Canvas healthChart=new(){Height=100};
    ComboBox? healthDrives,healthInterval,healthMode;
    Button? healthRefresh;
    JsonNode? healthData,healthDetail;
    DispatcherQueueTimer? healthTimer;
    long healthGeneration,healthRequest;
    bool healthRendering,healthFetching;
    static long HealthSeconds(JsonNode? value)=>double.TryParse(value?.ToString(),System.Globalization.NumberStyles.Float,System.Globalization.CultureInfo.InvariantCulture,out var n)?(long)n:0;
    bool HealthAvailable=>healthData?["probe_available"]?.ToString()=="true";
    string HealthValue(JsonNode? node,string suffix="")=>node==null||string.IsNullOrWhiteSpace(node.ToString())?"—":node.ToString()+suffix;
    async Task OpenDriveHealth(){if(!quick)await NavigateDestination(new JsonObject{["route"]="health",["nav"]="health",["title"]="Disk Health",["query"]="",["location"]=new JsonObject()});}
    void StopHealthPolling(){healthTimer?.Stop();healthGeneration++;}
    void BuildHealthPanel(){
        healthPanel=new Grid{Padding=new Thickness(14,0,14,8),RowSpacing=8};
        foreach(var height in new[]{GridLength.Auto,GridLength.Auto,GridLength.Auto,GridLength.Auto,GridLength.Auto,GridLength.Auto,new GridLength(1,GridUnitType.Star),GridLength.Auto})healthPanel.RowDefinitions.Add(new(){Height=height});
        void Add(FrameworkElement e,int row){Grid.SetRow(e,row);healthPanel.Children.Add(e);}
        var actions=new StackPanel{Orientation=Orientation.Horizontal,Spacing=8,Margin=new Thickness(0,5,0,0)};
        healthRefresh=FlatButton("Refresh health","\uE72C",async()=>{if(HealthAvailable)await api.Post("health/refresh",new JsonObject());await RefreshHealth();});actions.Children.Add(healthRefresh);
        actions.Children.Add(Label("Monitor",11));healthInterval=new ComboBox{ItemsSource=new[]{"Manual","Every minute","Every 5 minutes","Every 15 minutes","Every 30 minutes","Every hour"},MinWidth=140,MinHeight=30,SelectedIndex=0};
        healthInterval.SelectionChanged+=async(_,_)=>{if(healthRendering||route!="health"||healthData==null)return;await Guard(async()=>{var s=healthData["settings"]!.DeepClone().AsObject();s["interval_minutes"]=new[]{0,1,5,15,30,60}[Math.Max(0,healthInterval.SelectedIndex)];await api.Post("health/settings",s);await RefreshHealth();});};actions.Children.Add(healthInterval);
        actions.Children.Add(FlatButton("Alert thresholds","\uE713",HealthSettings,true));actions.Children.Add(FlatButton("Copy report","\uE8C8",CopyHealthReport,true));Add(actions,0);
        healthDrives=new ComboBox{HorizontalAlignment=HorizontalAlignment.Stretch,MinHeight=34};healthDrives.SelectionChanged+=(_,_)=>{if(healthRendering||route!="health"||healthDrives.SelectedItem is not ComboBoxItem item)return;location["drive"]=item.Tag?.ToString();DispatcherQueue.TryEnqueue(async()=>await Guard(RefreshHealth));};Add(healthDrives,1);
        healthDrives.DropDownClosed+=async(_,_)=>{if(route=="health")await Guard(RefreshHealth);};
        var heading=new StackPanel{Spacing=6};healthNotice.TextWrapping=TextWrapping.Wrap;healthNotice.Foreground=Brush(0xE5BD4Cu);heading.Children.Add(healthNotice);heading.Children.Add(healthTitle);healthSummary.Visibility=Visibility.Collapsed;heading.Children.Add(healthSummary);Add(heading,2);
        healthInfo.TextWrapping=TextWrapping.Wrap;healthInfo.Opacity=.6;var overview=new StackPanel{Spacing=8};overview.Children.Add(healthOverview);overview.Children.Add(healthInfo);Add(overview,3);
        var modes=new StackPanel{Orientation=Orientation.Horizontal,Spacing=10};healthMode=new ComboBox{ItemsSource=new[]{"S.M.A.R.T. attributes","Temperature history","Health alerts"},SelectedIndex=0,MinWidth=190,MinHeight=28};healthMode.SelectionChanged+=(_,_)=>{if(healthRendering)return;location["mode"]=healthMode.SelectedIndex;RenderHealthDetail();PersistTabs();};modes.Children.Add(healthMode);modes.Children.Add(healthChartLabel);Add(modes,4);
        healthChart.SizeChanged+=(_,_)=>RenderHealthChart();Add(healthChart,5);
        healthList.ItemsSource=healthRows;
        healthList.ItemTemplate=(DataTemplate)XamlReader.Load("<DataTemplate xmlns='http://schemas.microsoft.com/winfx/2006/xaml/presentation'><Grid MinHeight='30' ColumnSpacing='8'><Grid.ColumnDefinitions><ColumnDefinition Width='34'/><ColumnDefinition Width='*'/><ColumnDefinition Width='54'/><ColumnDefinition Width='54'/><ColumnDefinition Width='64'/><ColumnDefinition Width='170'/></Grid.ColumnDefinitions><TextBlock Text='{Binding Id}' Opacity='.6'/><TextBlock Grid.Column='1' Text='{Binding Name}' TextTrimming='CharacterEllipsis' ToolTipService.ToolTip='{Binding Name}'/><TextBlock Grid.Column='2' Text='{Binding Current}' HorizontalAlignment='Right'/><TextBlock Grid.Column='3' Text='{Binding Worst}' HorizontalAlignment='Right'/><TextBlock Grid.Column='4' Text='{Binding Threshold}' HorizontalAlignment='Right'/><TextBlock Grid.Column='5' Text='{Binding Raw}' HorizontalAlignment='Right' TextTrimming='CharacterEllipsis' ToolTipService.ToolTip='{Binding Raw}'/></Grid></DataTemplate>");
        var style=new Style(typeof(ListViewItem));style.Setters.Add(new Setter(Control.HorizontalContentAlignmentProperty,HorizontalAlignment.Stretch));style.Setters.Add(new Setter(Control.PaddingProperty,new Thickness(4,4,4,0)));style.Setters.Add(new Setter(FrameworkElement.MinHeightProperty,30d));style.Setters.Add(new Setter(Control.FontSizeProperty,11d));healthList.ItemContainerStyle=style;Add(healthList,6);
        healthNote.TextWrapping=TextWrapping.Wrap;healthNote.MaxLines=5;healthNote.Opacity=.8;Add(healthNote,7);
    }
    async Task ShowDriveHealth(){
        StopDiskPolling();StopHealthPolling();crawlTimer?.Stop();Interlocked.Increment(ref querySequence);rows.Clear();
        if(healthPanel==null)BuildHealthPanel();
        if(!page.Children.Contains(explorerChrome)){page.Children.Clear();page.Children.Add(explorerChrome);}
        foreach(var child in explorerChrome.Children.Skip(1))child.Visibility=Visibility.Collapsed;explorerViews.Visibility=Visibility.Collapsed;
        foreach(var child in page.Children.ToArray())if(child!=explorerChrome&&child!=healthPanel)page.Children.Remove(child);
        if(!page.Children.Contains(healthPanel!)){Grid.SetRow(healthPanel!,1);Grid.SetRowSpan(healthPanel!,2);page.Children.Add(healthPanel!);}
        currentTitle="Disk Health";SelectNavigation("health");healthDetail=null;healthData=null;healthRows.Clear();healthTitle.Text="Disk Health";healthSummary.Text="Reading saved health information…";healthNote.Text="";
        var generation=healthGeneration;await RefreshHealth();if(generation!=healthGeneration||route!="health")return;
        if(HealthAvailable&&(healthData?["drives"]?.AsArray().Count??0)==0&&healthData?["refreshing"]?.ToString()!="true"){if(HealthAvailable)await api.Post("health/refresh",new JsonObject());await RefreshHealth();}
        if(generation!=healthGeneration||route!="health")return;
        healthTimer??=DispatcherQueue.CreateTimer();healthTimer.Interval=TimeSpan.FromSeconds(2);healthTimer.Tick-=HealthTick;healthTimer.Tick+=HealthTick;healthTimer.Start();PersistTabs();
    }
    async void HealthTick(DispatcherQueueTimer timer,object args){if(!healthFetching&&route=="health")await Guard(RefreshHealth);}
    async Task RefreshHealth(){
        if(route!="health")return;healthFetching=true;var request=++healthRequest;var generation=healthGeneration;var selected=location["drive"]?.ToString();
        try{
            var data=await api.Get("health");if(generation!=healthGeneration||request!=healthRequest||route!="health")return;
            var drives=data["drives"]?.AsArray()??[];var drive=drives.FirstOrDefault(d=>d?["id"]?.ToString()==selected)??drives.FirstOrDefault(d=>d?["present"]?.ToString()=="true")??drives.FirstOrDefault();
            var id=drive?["id"]?.ToString();JsonNode? detail=id==null?null:await api.Post("health/detail",new JsonObject{["id"]=id});
            if(generation!=healthGeneration||request!=healthRequest||route!="health"||selected!=location["drive"]?.ToString())return;
            healthData=data;healthDetail=detail;healthRendering=true;
            if(id!=null)location["drive"]=id;UpdateHealthDriveChoices(drives,id);
            healthInterval!.SelectedIndex=Math.Max(0,Array.IndexOf(new[]{0,1,5,15,30,60},(int)Number(data["settings"]?["interval_minutes"])));healthMode!.SelectedIndex=Math.Clamp((int)Number(location["mode"]),0,2);
            healthRefresh!.IsEnabled=HealthAvailable&&data["refreshing"]?.ToString()!="true";healthInterval!.IsEnabled=HealthAvailable;ToolTipService.SetToolTip(healthRefresh,data["disabled_reason"]?.ToString());healthTimer?.Stop();if(healthTimer!=null){healthTimer.Interval=TimeSpan.FromSeconds(data["refreshing"]?.ToString()=="true"?2:15);healthTimer.Start();}
            RenderHealthDetail();var sampled=HealthSeconds(drive?["sampled_at"]);message.Text=$"Disk Health · {(data["refreshing"]?.ToString()=="true"?"Checking…":sampled>0?"Saved "+DateTimeOffset.FromUnixTimeSeconds(sampled).ToLocalTime().ToString("g"):"Ready")} · {(api.Mode=="client"?"Coordinator computer":"This computer")} · No AI calls";PersistTabs();
        }finally{healthRendering=false;if(request==healthRequest)healthFetching=false;}
    }
    void UpdateHealthDriveChoices(JsonArray drives,string? selected){
        // WinUI owns selection while the dropdown is open. Replacing its items
        // there can invalidate the native selection container and terminate XAML.
        if(healthDrives==null||healthDrives.IsDropDownOpen)return;
        var values=drives.Where(d=>d!=null).Select(d=>(Id:d!["id"]?.ToString(),Label:$"{d["model"]} · {d["bus"]} · {d["status"]}"+(d["present"]?.ToString()=="false"?" · Disconnected":""))).ToArray();
        var existing=healthDrives.Items.OfType<ComboBoxItem>().ToArray();
        if(!existing.Select(i=>i.Tag?.ToString()).SequenceEqual(values.Select(v=>v.Id))){
            healthDrives.Items.Clear();foreach(var value in values)healthDrives.Items.Add(new ComboBoxItem{Tag=value.Id,Content=value.Label});
            existing=healthDrives.Items.OfType<ComboBoxItem>().ToArray();
        }
        for(int i=0;i<existing.Length;i++){if(existing[i].Content?.ToString()!=values[i].Label)existing[i].Content=values[i].Label;}
        var next=existing.FirstOrDefault(i=>i.Tag?.ToString()==selected);if(!ReferenceEquals(healthDrives.SelectedItem,next))healthDrives.SelectedItem=next;
    }
    void RenderHealthDetail(){
        RenderHealthOverview();
        var d=healthDetail?["drive"];healthNotice.Text=healthData?["fixture_only"]?.ToString()=="true"?"SYNTHETIC PREVIEW · No data from your drives":!HealthAvailable?"Live checks disabled · Saved reports only":"";healthRows.Clear();healthChart.Visibility=healthMode?.SelectedIndex==1?Visibility.Visible:Visibility.Collapsed;
        if(d==null){healthTitle.Text="Disk Health";healthSummary.Text=healthData?["refreshing"]?.ToString()=="true"?"Checking connected drives…":"No saved readings · Live checks disabled";healthInfo.Text="";healthNote.Text=healthData?["error"]?.ToString()??"";healthChart.Children.Clear();return;}
        var current=healthData?["drives"]?.AsArray().FirstOrDefault(v=>v?["id"]?.ToString()==d["id"]?.ToString());var status=d["status"]?.ToString()??"Unknown";
        healthTitle.Text=d["model"]?.ToString()??"Drive";
        healthSummary.Text=$"{status}  ·  {HealthValue(d["temperature_c"]," °C")}  ·  {HealthValue(d["remaining_percent"],"% rated endurance remaining")}"+(current?["present"]?.ToString()=="false"?"  ·  Disconnected":current?["stale"]?.ToString()=="true"?"  ·  Saved reading":"");
        healthSummary.Foreground=Brush(status=="Bad"?0xEC807Au:status=="Caution"?0xE5BD4Cu:status=="Good"?0x89B89Au:0xA0A0A0u);
        string Bytes(JsonNode? v)=>long.TryParse(v?.ToString(),out var n)?DiskBytes(n):"—";
        healthInfo.Text=$"{d["bus"]} · {Bytes(d["capacity_bytes"])} · Firmware {HealthValue(d["firmware"])}\nPower on: {HealthValue(d["power_on_hours"]," h")} · Power cycles: {HealthValue(d["power_cycles"])} · Host reads: {Bytes(d["host_reads_bytes"])} · Host writes: {Bytes(d["host_writes_bytes"])}\n{d["source"]}";
        if(HealthSeconds(d["sampled_at"])>0)healthInfo.Text+=" · Saved "+DateTimeOffset.FromUnixTimeSeconds(HealthSeconds(d["sampled_at"])).ToLocalTime().ToString("g");
        healthChartLabel.Text=healthMode?.SelectedIndex==0?"Current / Worst / Threshold / Raw":healthMode?.SelectedIndex==1?"Latest 180 samples · °C":"Health transitions";
        if(healthMode?.SelectedIndex==0)foreach(var a in d["attributes"]?.AsArray()??[]){if(a==null)continue;healthRows.Add(new(a["id"]?.ToString()??"",a["name"]?.ToString()??"",HealthValue(a["current"]),HealthValue(a["worst"]),HealthValue(a["threshold"]),HealthValue(a["raw"])+" "+a["unit"]));}
        if(healthMode?.SelectedIndex==1)foreach(var h in (healthDetail?["history"]?.AsArray()??[]).Reverse()){if(h==null)continue;healthRows.Add(new("",DateTimeOffset.FromUnixTimeSeconds(HealthSeconds(h["time"])).ToLocalTime().ToString("g"),"","","",HealthValue(h["temperature_c"]," °C")+" · "+HealthValue(h["remaining_percent"],"%")));}
        if(healthMode?.SelectedIndex==2)foreach(var h in healthDetail?["events"]?.AsArray()??[]){if(h==null)continue;healthRows.Add(new("",string.Join(" ",h["reasons"]?.AsArray().Select(v=>v?.ToString())??[]),"","","",h["status"]+" · "+DateTimeOffset.FromUnixTimeSeconds(HealthSeconds(h["time"])).ToLocalTime().ToString("g")));}
        var notes=new List<string>();if(!string.IsNullOrEmpty(healthData?["error"]?.ToString()))notes.Add(healthData!["error"]!.ToString());notes.AddRange(d["reasons"]?.AsArray().Select(v=>v?.ToString()??"")??[]);notes.AddRange(d["limitations"]?.AsArray().Select(v=>v?.ToString()??"")??[]);healthNote.Text=string.Join("\n",notes);RenderHealthOverview();RenderHealthChart();
    }
    void RenderHealthChart(){
        healthChart.Children.Clear();if(healthMode?.SelectedIndex!=1)return;var samples=(healthDetail?["history"]?.AsArray()??[]).ToArray();var valid=samples.Where(s=>s?["temperature_c"]!=null).ToArray();if(valid.Length<2){healthChart.Children.Add(Label("Two measured samples are needed for a trend. Monitoring records readings while Retriever is running.",11));return;}
        double width=Math.Max(1,healthChart.ActualWidth-8),min=valid.Min(v=>Number(v?["temperature_c"]))-2,max=valid.Max(v=>Number(v?["temperature_c"]))+2,first=HealthSeconds(samples.First()?["time"]),last=HealthSeconds(samples.Last()?["time"]);Polyline? line=null;
        foreach(var s in samples){if(s?["temperature_c"]==null){line=null;continue;}if(line==null){line=new Polyline{Stroke=Brush(0xE5BD4C),StrokeThickness=2};healthChart.Children.Add(line);}line.Points.Add(new Windows.Foundation.Point(4+(HealthSeconds(s["time"])-first)/Math.Max(1,last-first)*width,90-(Number(s["temperature_c"])-min)/(max-min)*80));}
    }
    Task HealthSettings(){
        if(healthData==null)return Task.CompletedTask;var settings=healthData["settings"]!;
        var temperature=new NumberBox{Header="Temperature alert · °C",Value=Number(settings["temperature_c"]),Minimum=30,Maximum=90,SpinButtonPlacementMode=NumberBoxSpinButtonPlacementMode.Compact};
        var life=new NumberBox{Header="Endurance remaining · %",Value=Number(settings["remaining_life_percent"]),Minimum=0,Maximum=50,SpinButtonPlacementMode=NumberBoxSpinButtonPlacementMode.Compact};
        var content=new StackPanel{Spacing=10};content.Children.Add(temperature);content.Children.Add(life);content.Children.Add(new TextBlock{Text="Alert thresholds only. Drive firmware is untouched.",FontSize=10,Opacity=.6,TextWrapping=TextWrapping.Wrap});
        content.Children.Add(FlatButton("Save thresholds","\uE73E",async()=>{if(!double.IsFinite(temperature.Value)||!double.IsFinite(life.Value))throw new InvalidOperationException("Enter both thresholds.");await api.Post("health/settings",new JsonObject{["interval_minutes"]=HealthAvailable?settings["interval_minutes"]?.DeepClone():JsonValue.Create(0),["temperature_c"]=(int)temperature.Value,["remaining_life_percent"]=(int)life.Value});activePopover?.Hide();await RefreshHealth();}));
        Popover("Health alerts",content,width:300);return Task.CompletedTask;
    }
    Task CopyHealthReport(){if(healthDetail==null)return Task.CompletedTask;var data=new DataPackage();data.SetText(healthDetail.ToJsonString(new System.Text.Json.JsonSerializerOptions{WriteIndented=true}));Clipboard.SetContent(data);message.Text="Disk Health report copied. Includes drive identity, readings, history and alerts.";return Task.CompletedTask;}
}
