using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Shapes;
using System.Text.Json.Nodes;

namespace Retriever.Native;

public sealed partial class MainWindow {
    readonly Grid capacityOverview=new(){ColumnSpacing=10,Height=142,Margin=new Thickness(0,0,0,12)},healthOverview=new(){ColumnSpacing=10,Height=122};
    static readonly uint[] ChartColors=[0xe5bd4c,0x8fb6a0,0x9caad3,0xc09caf,0x82b9c4];
    Border InsightCard(string heading,UIElement content){var body=new StackPanel{Spacing=10};var label=Label(heading.ToUpperInvariant(),10);label.Opacity=.6;body.Children.Add(label);body.Children.Add(content);return new Border{Background=chrome,BorderBrush=line,BorderThickness=new Thickness(1),CornerRadius=new CornerRadius(10),Padding=new Thickness(14,12,14,10),Child=body};}
    static TextBlock Metric(string value,uint? color=null){var label=new TextBlock{Text=value,FontSize=26,FontWeight=Microsoft.UI.Text.FontWeights.SemiBold,TextTrimming=TextTrimming.CharacterEllipsis};if(color.HasValue)label.Foreground=Brush(color.Value);return label;}
    UIElement Ring(double? fraction,string value,string caption,uint color,string ringLabel="left"){
        var grid=new Grid{ColumnSpacing=12};grid.ColumnDefinitions.Add(new(){Width=new GridLength(82)});grid.ColumnDefinitions.Add(new(){Width=new GridLength(1,GridUnitType.Star)});
        var canvas=new Canvas{Width=78,Height=78};canvas.Children.Add(new Ellipse{Width=70,Height=70,Stroke=line,StrokeThickness=6,Margin=new Thickness(4)});
        if(fraction is double f&&double.IsFinite(f)){
            var points=new PointCollection();var n=Math.Clamp(f,0,1);for(int i=0;i<=100*n;i++){var a=(i/100d*360-90)*Math.PI/180;points.Add(new(39+35*Math.Cos(a),39+35*Math.Sin(a)));}canvas.Children.Add(new Polyline{Points=points,Stroke=Brush(color),StrokeThickness=6,StrokeStartLineCap=PenLineCap.Round,StrokeEndLineCap=PenLineCap.Round});
        }
        var center=Label(fraction.HasValue?(Math.Clamp(fraction.Value,0,1)*100).ToString("0")+"%":"—",17);center.Width=78;center.TextAlignment=TextAlignment.Center;Canvas.SetTop(center,20);canvas.Children.Add(center);var meaning=Label(fraction.HasValue?ringLabel:"unknown",9);meaning.Width=78;meaning.TextAlignment=TextAlignment.Center;meaning.Opacity=.6;Canvas.SetTop(meaning,43);canvas.Children.Add(meaning);AutomationProperties.SetName(canvas,center.Text+" "+ringLabel);
        var scaled=new Viewbox{Child=canvas,Width=78,Height=78};grid.Children.Add(scaled);
        var labels=new StackPanel{Spacing=5,VerticalAlignment=VerticalAlignment.Center};var metric=Metric(value);labels.Children.Add(metric);labels.Children.Add(new TextBlock{Text=caption,FontSize=10,Opacity=.6,TextWrapping=TextWrapping.Wrap,MaxLines=3});Grid.SetColumn(labels,1);grid.Children.Add(labels);
        grid.SizeChanged+=(_,_)=>{bool compact=grid.ActualWidth<220;scaled.Width=scaled.Height=compact?54:78;grid.ColumnDefinitions[0].Width=new GridLength(compact?58:82);metric.FontSize=compact?18:26;};return grid;
    }
    void RenderCapacityOverview(){
        capacityOverview.Children.Clear();capacityOverview.ColumnDefinitions.Clear();foreach(var width in new[]{1.1,1,1.1})capacityOverview.ColumnDefinitions.Add(new(){Width=new GridLength(width,GridUnitType.Star)});
        var scan=diskResult?["scan"];var totals=scan?["state"]?.ToString()=="scanning"?scan?["summary"]?["totals"]:diskResult?["current"];var volume=scan?["summary"]?["volume"];double total=Number(volume?["total"]),free=Number(volume?["available"]);
        var used=Math.Max(0,total-free);var volumeCard=InsightCard("Volume capacity",Ring(total>0?used/total:null,total>0?DiskBytes((long)free):"—",total>0?"available of "+DiskBytes((long)total):"Choose a folder to begin",ChartColors[0],"used"));capacityOverview.Children.Add(volumeCard);
        var summary=new StackPanel{Spacing=4};summary.Children.Add(Metric(scan==null?"—":DiskBytes(Number(totals?["logical"]))));summary.Children.Add(Label(scan==null?"No saved scan":$"{Number(totals?["files"]):N0} files · {DiskBytes(Number(totals?["allocated"]))} allocated",10));summary.Children.Add(new TextBlock{Text=diskResult?["previous_scan"]==null?"First snapshot · scan again to compare":"Snapshot comparison available in Size changes",FontSize=10,Opacity=.55,TextWrapping=TextWrapping.Wrap});var selected=InsightCard("Selected folder",summary);Grid.SetColumn(selected,1);capacityOverview.Children.Add(selected);
        var types=new StackPanel{Spacing=6};var data=(diskResult?["types"]?.AsArray()??[]).Take(4).ToArray();double sum=(diskResult?["types"]?.AsArray()??[]).Sum(t=>(double)Number(t?["logical"]));
        for(int i=0;i<data.Length;i++){var item=data[i];var row=new Grid{ColumnSpacing=8};row.ColumnDefinitions.Add(new(){Width=new GridLength(40)});row.ColumnDefinitions.Add(new(){Width=new GridLength(1,GridUnitType.Star)});row.ColumnDefinitions.Add(new(){Width=new GridLength(64)});var ext=item?["extension"]?.ToString()??"";row.Children.Add(Label(ext.Length>0?ext.ToUpperInvariant():"Other",10));var bar=new ProgressBar{Value=sum>0?Number(item?["logical"])/sum*100:0,Maximum=100,Foreground=Brush(ChartColors[i]),Height=4,VerticalAlignment=VerticalAlignment.Center};AutomationProperties.SetName(bar,ext+" share of scanned file bytes");Grid.SetColumn(bar,1);row.Children.Add(bar);var bytes=Label(DiskBytes(Number(item?["logical"])),10);bytes.HorizontalAlignment=HorizontalAlignment.Right;Grid.SetColumn(bytes,2);row.Children.Add(bytes);types.Children.Add(row);}
        if(data.Length==0)types.Children.Add(new TextBlock{Text="File types will appear after your first scan.",FontSize=11,Opacity=.55,TextWrapping=TextWrapping.Wrap});var distribution=InsightCard("File type breakdown",types);Grid.SetColumn(distribution,2);capacityOverview.Children.Add(distribution);
    }
    void RenderHealthOverview(){
        healthOverview.Children.Clear();healthOverview.ColumnDefinitions.Clear();for(int i=0;i<3;i++)healthOverview.ColumnDefinitions.Add(new(){Width=new GridLength(1,GridUnitType.Star)});
        var drive=healthDetail?["drive"];var state=drive?["status"]?.ToString()??"Unknown";uint color=state=="Good"?0x8fb6a0u:state=="Bad"?0xe39c8eu:state=="Caution"?0xe5bd4cu:0xa4a4a4u;
        var status=new StackPanel{Spacing=5};status.Children.Add(Metric(state,color));status.Children.Add(new TextBlock{Text=drive==null?"No saved telemetry":(drive["stale"]?.ToString()=="true"?"Saved reading · ":"")+(drive["present"]?.ToString()=="false"?"Disconnected":"SMART assessment"),FontSize=11,Opacity=.6,TextWrapping=TextWrapping.Wrap});healthOverview.Children.Add(InsightCard("Reported health",status));
        var temperature=new StackPanel{Spacing=1};temperature.Children.Add(Metric(HealthValue(drive?["temperature_c"]," °C")));var spark=new Canvas{Height=32};spark.SizeChanged+=(_,_)=>DrawTemperature(spark,32);temperature.Children.Add(spark);var tempCard=InsightCard("Temperature · saved history",temperature);Grid.SetColumn(tempCard,1);healthOverview.Children.Add(tempCard);
        double? remaining=double.TryParse(drive?["remaining_percent"]?.ToString(),out var value)?Math.Clamp(value,0,100):null;var endurance=InsightCard("Rated endurance",Ring(remaining/100,remaining.HasValue?remaining.Value.ToString("0")+"%":"—",remaining.HasValue?"remaining · not a failure prediction":"Not reported by this drive",color));Grid.SetColumn(endurance,2);healthOverview.Children.Add(endurance);
    }
    void DrawTemperature(Canvas canvas,double height){
        canvas.Children.Clear();var samples=(healthDetail?["history"]?.AsArray()??[]).ToArray();var valid=samples.Where(v=>v?["temperature_c"]!=null).ToArray();if(valid.Length<2){var label=Label("No measured trend yet",10);label.Opacity=.5;canvas.Children.Add(label);return;}
        double low=valid.Min(v=>Number(v?["temperature_c"]))-2,high=valid.Max(v=>Number(v?["temperature_c"]))+2,first=HealthSeconds(samples.First()?["time"]),last=HealthSeconds(samples.Last()?["time"]),width=Math.Max(1,canvas.ActualWidth-8);Polyline? segment=null;
        foreach(var sample in samples){if(sample?["temperature_c"]==null){segment=null;continue;}if(segment==null){segment=new Polyline{Stroke=accent,StrokeThickness=2};canvas.Children.Add(segment);}segment.Points.Add(new(4+(HealthSeconds(sample["time"])-first)/Math.Max(1,last-first)*width,height-4-(Number(sample["temperature_c"])-low)/(high-low)*(height-8)));}
        AutomationProperties.SetName(canvas,$"Temperature history, {valid.Length} measured samples, {low+2:0} to {high-2:0} degrees Celsius. Missing samples are gaps.");
    }
    Task SortFiles(){
        var body=new StackPanel{Spacing=3};foreach(var (key,name) in new[]{("name","Name"),("modified","Date modified"),("size","Size"),("type","Type"),("priority","Priority")})body.Children.Add(FlatButton(name,sort==key?"\uE73E":"",async()=>{activePopover?.Hide();await SortBy(key);}));body.Children.Add(new TextBlock{Text="Select the current property again to reverse order.",FontSize=10,Opacity=.55,TextWrapping=TextWrapping.Wrap});Popover("Sort files",body,sortButton,250);return Task.CompletedTask;
    }
}
