using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Hosting;
using Microsoft.UI.Xaml.Markup;
using Microsoft.UI.Xaml.Media;
using System.Numerics;
using System.Text.Json.Nodes;
using Windows.System;

namespace Retriever.Native;

public sealed partial class MainWindow {
    Grid? searchAnchor;
    Border? searchExpansion;
    ListView? expandedResults;
    readonly StackPanel expandedPreview=new(){Spacing=10};
    readonly TextBlock expandedStatus=new(){FontSize=11,Opacity=.65,TextWrapping=TextWrapping.Wrap};
    CancellationTokenSource? expandedPreviewRequest;
    string searchMode="hybrid";
    bool searchExpanded=>searchExpansion?.Visibility==Visibility.Visible;

    void SetupSearchExpansion(Grid anchor){
        searchAnchor=anchor;
        search.AddHandler(UIElement.PointerPressedEvent,new Microsoft.UI.Xaml.Input.PointerEventHandler((_,_)=>DispatcherQueue.TryEnqueue(OpenSearchExpansion)),true);
        root.AddHandler(UIElement.PointerPressedEvent,new Microsoft.UI.Xaml.Input.PointerEventHandler((_,e)=>{
            if(!searchExpanded)return;
            for(var node=e.OriginalSource as DependencyObject;node!=null;node=VisualTreeHelper.GetParent(node))if(node==searchAnchor||node==searchExpansion)return;
            CloseSearchExpansion();
        }),true);
        search.PreviewKeyDown+=async(_,e)=>{
            if(!searchExpanded)return;
            if(e.Key==VirtualKey.Escape){e.Handled=true;CloseSearchExpansion();}
            else if(e.Key is VirtualKey.Down or VirtualKey.Up){e.Handled=true;if(rows.Count>0){expandedResults!.SelectedIndex=Math.Clamp(expandedResults.SelectedIndex+(e.Key==VirtualKey.Down?1:-1),0,rows.Count-1);expandedResults.ScrollIntoView(expandedResults.SelectedItem);}}
            else if(e.Key==VirtualKey.Enter&&expandedResults?.SelectedItem is FileRow selected){e.Handled=true;files.SelectedItem=selected;CloseSearchExpansion();await Guard(OpenFile);}
        };
        root.SizeChanged+=(_,_)=>PositionSearchExpansion();
    }
    void OpenSearchExpansion(){
        if(quick||config==null||dialogVisible)return;
        if(searchExpansion==null){
            var panel=new Grid{RowSpacing=12,Padding=new Thickness(16)};panel.RowDefinitions.Add(new(){Height=GridLength.Auto});panel.RowDefinitions.Add(new(){Height=new GridLength(1,GridUnitType.Star)});panel.RowDefinitions.Add(new(){Height=GridLength.Auto});
            var top=new Grid();top.ColumnDefinitions.Add(new(){Width=new GridLength(1,GridUnitType.Star)});top.ColumnDefinitions.Add(new(){Width=GridLength.Auto});
            var modes=new StackPanel{Orientation=Orientation.Horizontal,Spacing=5};
            foreach(var (key,label) in new[]{("hybrid","Smart search"),("exact","Words"),("semantic","Meaning")})modes.Children.Add(FlatButton(label,"",async()=>{searchMode=key;expandedStatus.Text=label;if(search.Text.Trim().Length>0)await Search(true);}));
            top.Children.Add(modes);var close=FlatButton("Close search details","\uE711",()=>{CloseSearchExpansion();return Task.CompletedTask;},true);Grid.SetColumn(close,1);top.Children.Add(close);panel.Children.Add(top);
            var body=new Grid{ColumnSpacing=16};body.ColumnDefinitions.Add(new(){Width=new GridLength(1,GridUnitType.Star)});body.ColumnDefinitions.Add(new(){Width=new GridLength(1,GridUnitType.Star)});
            expandedResults=new ListView{ItemsSource=rows,SelectionMode=ListViewSelectionMode.Single};
            expandedResults.ItemTemplate=(DataTemplate)XamlReader.Load("<DataTemplate xmlns='http://schemas.microsoft.com/winfx/2006/xaml/presentation'><Grid Padding='4,6' ColumnSpacing='9'><Grid.ColumnDefinitions><ColumnDefinition Width='22'/><ColumnDefinition Width='*'/></Grid.ColumnDefinitions><Image Source='{Binding Icon}' Width='20' Height='20'/><StackPanel Grid.Column='1' Spacing='4'><TextBlock Text='{Binding Name}' FontSize='12' TextTrimming='CharacterEllipsis'/><TextBlock Text='{Binding Detail}' FontSize='10' Opacity='.6' TextTrimming='CharacterEllipsis'/></StackPanel></Grid></DataTemplate>");
            expandedResults.SelectionChanged+=async(_,_)=>await PreviewExpandedResult();expandedResults.DoubleTapped+=async(_,_)=>{if(expandedResults.SelectedItem is FileRow selected){files.SelectedItem=selected;CloseSearchExpansion();await Guard(OpenFile);}};
            body.Children.Add(expandedResults);var previewScroll=new ScrollViewer{Content=expandedPreview,Padding=new Thickness(12,0,0,0),BorderBrush=line,BorderThickness=new Thickness(1,0,0,0)};Grid.SetColumn(previewScroll,1);body.Children.Add(previewScroll);Grid.SetRow(body,1);panel.Children.Add(body);
            Grid.SetRow(expandedStatus,2);panel.Children.Add(expandedStatus);
            searchExpansion=new Border{Child=panel,Background=chrome,BorderBrush=line,BorderThickness=new Thickness(1),CornerRadius=new CornerRadius(12),HorizontalAlignment=HorizontalAlignment.Left,VerticalAlignment=VerticalAlignment.Top,Visibility=Visibility.Collapsed};Grid.SetRowSpan(searchExpansion,3);root.Children.Add(searchExpansion);
        }
        searchExpansion.Visibility=Visibility.Visible;PositionSearchExpansion();expandedStatus.Text=search.Text.Trim().Length==0?"Search indexed folders by filename, words, or meaning. ↑ ↓ Select · Enter Open · Esc Close":message.Text;
        expandedResults!.Visibility=search.Text.Trim().Length==0?Visibility.Collapsed:Visibility.Visible;
        if(search.Text.Trim().Length==0){expandedPreview.Children.Clear();expandedPreview.Children.Add(Label("Your files, with context",17));expandedPreview.Children.Add(new TextBlock{Text="Start with a filename, a phrase from a document, or describe what you remember. Smart search uses the index first and calls models only when useful.",TextWrapping=TextWrapping.Wrap,Opacity=.7});}
        else if(rows.Count>0)expandedResults.SelectedIndex=0;
        ElementCompositionPreview.SetIsTranslationEnabled(searchExpansion,true);
        var visual=ElementCompositionPreview.GetElementVisual(searchExpansion);if(new Windows.UI.ViewManagement.UISettings().AnimationsEnabled){var fade=visual.Compositor.CreateScalarKeyFrameAnimation();fade.InsertKeyFrame(0,0);fade.InsertKeyFrame(1,1);fade.Duration=TimeSpan.FromMilliseconds(170);visual.StartAnimation("Opacity",fade);var move=visual.Compositor.CreateVector3KeyFrameAnimation();move.InsertKeyFrame(0,new Vector3(0,-6,0));move.InsertKeyFrame(1,Vector3.Zero);move.Duration=TimeSpan.FromMilliseconds(210);visual.StartAnimation("Translation",move);}
    }
    void PositionSearchExpansion(){if(!searchExpanded||searchAnchor==null)return;var position=searchAnchor.TransformToVisual(root).TransformPoint(new(0,searchAnchor.ActualHeight));searchExpansion!.Width=Math.Min(Math.Max(520,searchAnchor.ActualWidth),Math.Max(280,root.ActualWidth-24));searchExpansion.Height=Math.Max(180,Math.Min(390,root.ActualHeight-position.Y-24));searchExpansion.Margin=new Thickness(Math.Max(12,Math.Min(position.X,root.ActualWidth-searchExpansion.Width-12)),position.Y+6,0,0);}
    void CloseSearchExpansion(){if(searchExpansion!=null)searchExpansion.Visibility=Visibility.Collapsed;expandedPreviewRequest?.Cancel();}
    void UpdateExpandedSearch(){if(!searchExpanded)return;expandedResults!.Visibility=Visibility.Visible;expandedStatus.Text=message.Text;if(rows.Count>0)expandedResults.SelectedIndex=0;else{expandedPreview.Children.Clear();expandedPreview.Children.Add(new TextBlock{Text="No matches yet. Try a different word or search mode.",TextWrapping=TextWrapping.Wrap,Opacity=.6});}}
    async Task PreviewExpandedResult(){
        expandedPreviewRequest?.Cancel();expandedPreviewRequest=new();var token=expandedPreviewRequest.Token;expandedPreview.Children.Clear();if(expandedResults?.SelectedItem is not FileRow file||!searchExpanded)return;
        expandedPreview.Children.Add(new TextBlock{Text=file.Name,FontSize=15,FontWeight=Microsoft.UI.Text.FontWeights.SemiBold,TextWrapping=TextWrapping.Wrap});expandedPreview.Children.Add(new TextBlock{Text=file.Path,FontSize=10,Opacity=.55,TextWrapping=TextWrapping.Wrap});
        var text=new TextBlock{Text="Loading preview…",FontSize=12,TextWrapping=TextWrapping.Wrap,IsTextSelectionEnabled=true};expandedPreview.Children.Add(text);
        try{var result=await api.Post("workspace/preview",new JsonObject{["device"]=file.Device,["path"]=file.Path},cancel:token);if(!token.IsCancellationRequested)text.Text=result["text"]?.ToString()??file.Detail;}catch(OperationCanceledException){}catch(Exception e){if(!token.IsCancellationRequested)text.Text=e.Message;}
    }
}
