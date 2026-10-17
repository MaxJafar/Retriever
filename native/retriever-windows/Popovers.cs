using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Animation;
using System.Text.Json.Nodes;
using Windows.System;

namespace Retriever.Native;

public sealed partial class MainWindow {
    FrameworkElement? lastActionAnchor;
    Flyout? activePopover;
    readonly List<Flyout> openPopovers=[];
    JsonArray propertyDefinitions=[];
    string[] workspaceTags=[];
    readonly System.Runtime.CompilerServices.ConditionalWeakTable<Button,object> propertyCellHooks=new();
    void WirePropertyCells(DependencyObject parent){
        if(parent is Button button&&button.Tag is string tag&&tag.StartsWith("cell:")&&!propertyCellHooks.TryGetValue(button,out _)){
            propertyCellHooks.Add(button,new());button.Click+=async(_,_)=>{if(button.DataContext is FileRow file&&button.Tag is string key)await Guard(()=>EditFileProperty(file,key[5..],button));};
            button.SetBinding(Microsoft.UI.Xaml.Controls.Button.ContentProperty,new Microsoft.UI.Xaml.Data.Binding{Path=new PropertyPath("Metadata"),Converter=new PropertyCellConverter(),ConverterParameter=tag[5..]});
        }
        for(int i=0;i<VisualTreeHelper.GetChildrenCount(parent);i++)WirePropertyCells(VisualTreeHelper.GetChild(parent,i));
    }
    Flyout Popover(string title,UIElement content,FrameworkElement? anchor=null,double width=300){
        var flyout=new Flyout{Placement=FlyoutPlacementMode.BottomEdgeAlignedLeft,AreOpenCloseAnimationsEnabled=new Windows.UI.ViewManagement.UISettings().AnimationsEnabled};
        var panel=new StackPanel{Width=width,Spacing=12,RequestedTheme=root.RequestedTheme,Background=chrome};
        var heading=new Grid();heading.ColumnDefinitions.Add(new(){Width=new GridLength(1,GridUnitType.Star)});heading.ColumnDefinitions.Add(new(){Width=GridLength.Auto});
        var label=Label(title,12);label.FontWeight=Microsoft.UI.Text.FontWeights.SemiBold;heading.Children.Add(label);
        var close=FlatButton("Close "+title,"\uE711",()=>{flyout.Hide();return Task.CompletedTask;},true);Grid.SetColumn(close,1);heading.Children.Add(close);panel.Children.Add(heading);panel.Children.Add(content);
        flyout.Content=panel;var style=new Style(typeof(FlyoutPresenter));style.Setters.Add(new Setter(Control.PaddingProperty,new Thickness(14)));style.Setters.Add(new Setter(Control.CornerRadiusProperty,new CornerRadius(10)));style.Setters.Add(new Setter(Control.BorderThicknessProperty,new Thickness(1)));style.Setters.Add(new Setter(Control.BorderBrushProperty,line));style.Setters.Add(new Setter(Control.BackgroundProperty,chrome));style.Setters.Add(new Setter(FrameworkElement.MaxHeightProperty,Math.Max(240,root.ActualHeight-100)));flyout.FlyoutPresenterStyle=style;
        flyout.Opened+=(_,_)=>{if(!openPopovers.Contains(flyout))openPopovers.Add(flyout);dialogVisible=true;activePopover=flyout;DispatcherQueue.TryEnqueue(()=>{if(activePopover==flyout&&content is not ScrollViewer)FirstPopoverInput(content)?.Focus(FocusState.Keyboard);});};
        flyout.Closed+=(_,_)=>{openPopovers.Remove(flyout);dialogVisible=openPopovers.Count>0;activePopover=openPopovers.LastOrDefault();};
        // A previous property cell may have been virtualized away after a search.
        var target=new[]{anchor,lastActionAnchor,filterButton,search}.FirstOrDefault(e=>e is {IsLoaded:true}&&e.XamlRoot==root.XamlRoot)??search;
        flyout.ShowAt(target);return flyout;
    }
    static TextBox? FirstPopoverInput(DependencyObject parent){if(parent is TextBox input&&input.IsEnabled&&input.Visibility==Visibility.Visible)return input;for(int i=0;i<VisualTreeHelper.GetChildrenCount(parent);i++){var found=FirstPopoverInput(VisualTreeHelper.GetChild(parent,i));if(found!=null)return found;}return null;}
    Task ViewSettings(){
        var body=new StackPanel{Spacing=5};
        body.Children.Add(Label("Layout",11));
        var layout=new StackPanel{Orientation=Orientation.Horizontal,Spacing=6};layout.Children.Add(FlatButton("Table","\uE8FD",()=>SetView(false)));layout.Children.Add(FlatButton("Icons","\uE80A",()=>SetView(true)));body.Children.Add(layout);
        body.Children.Add(new Border{Height=1,Background=line,Margin=new Thickness(0,6,0,6)});
        body.Children.Add(Label("Property visibility",11));
        foreach(var (key,name) in FileColumnChoices()){
            var box=new CheckBox{Content=name,IsChecked=visibleColumns.Contains(key),MinHeight=30,FontSize=12};
            box.Click+=(_,_)=>{if(box.IsChecked==true)visibleColumns.Add(key);else visibleColumns.Remove(key);BuildFileColumns();PersistTabs();if(key=="duration"&&box.IsChecked==true)_ = ReadVisibleLengths();};body.Children.Add(box);
        }
        var sum=new CheckBox{Content="Sum measured lengths in footer",IsChecked=sumLengths,FontSize=12};sum.Click+=(_,_)=>{sumLengths=sum.IsChecked==true;UpdatePager();PersistTabs();};body.Children.Add(sum);
        var add=FlatButton("New property","\uE710",async()=>{await ClosePopover();await CreatePropertyPopover(columnsButton);});body.Children.Add(add);
        Popover("View settings",new ScrollViewer{Content=body,MaxHeight=470},columnsButton,width:280);return Task.CompletedTask;
    }
    IEnumerable<(string Key,string Name)> FileColumnChoices(){
        foreach(var c in new[]{("modified","Date modified"),("type","Type"),("size","Size"),("duration","Length"),("device","Device"),("priority","Priority"),("tags","Tags"),("status","Status")})yield return c;
        foreach(var d in propertyDefinitions)if(d!=null)yield return ("prop:"+d["id"],d["name"]!.ToString());
    }
    async Task LoadPropertyDefinitions(){var data=await api.Get("workspace/properties");propertyDefinitions=data["properties"]?.DeepClone().AsArray()??[];workspaceTags=(data["tags"]?.AsArray()??[]).Select(v=>v!.ToString()).ToArray();}
    async Task<JsonNode> AddPropertyOption(string id,string option){
        await LoadPropertyDefinitions();var definition=propertyDefinitions.First(d=>d?["id"]?.ToString()==id)!.DeepClone();var choices=definition["options"]!.AsArray();
        var existing=choices.FirstOrDefault(v=>v!.ToString().Equals(option,StringComparison.OrdinalIgnoreCase));if(existing!=null)return definition;
        choices.Add(option);var saved=await api.Post("workspace/property",definition);await LoadPropertyDefinitions();return saved;
    }
    async Task ClosePopover(){var popup=activePopover;if(popup==null)return;var closed=new TaskCompletionSource();void Done(object? sender,object args){popup.Closed-=Done;closed.TrySetResult();}popup.Closed+=Done;popup.Hide();await closed.Task;}
    Task CreatePropertyPopover()=>CreatePropertyPopover(null);
    Task CreatePropertyPopover(FrameworkElement? anchor){
        var panel=new StackPanel{Spacing=10};var name=new TextBox{PlaceholderText="Property name",MaxLength=60};panel.Children.Add(name);
        bool creating=false;
        var error=new TextBlock{TextWrapping=TextWrapping.Wrap,Foreground=Brush(0xe5a088),FontSize=11};
        panel.Children.Add(Label("Select a type",11));
        foreach(var (kind,label,glyph) in new[]{("text","Text","\uE8D2"),("number","Number","\uE8EF"),("select","Select","\uE73A"),("multi_select","Multi-select","\uE762"),("date","Date","\uE787"),("checkbox","Checkbox","\uE73D")}){
            var button=FlatButton(label,glyph,async()=>{if(creating)return;creating=true;try{var d=await api.Post("workspace/property",new JsonObject{["name"]=name.Text.Trim(),["kind"]=kind,["options"]=new JsonArray()});visibleColumns.Add("prop:"+d["id"]);await LoadPropertyDefinitions();BuildFileColumns();PersistTabs();activePopover?.Hide();message.Text="Property created. Select a cell to add its first value.";}catch(Exception e){error.Text=e.Message;}finally{creating=false;}});button.HorizontalAlignment=HorizontalAlignment.Stretch;button.HorizontalContentAlignment=HorizontalAlignment.Left;panel.Children.Add(button);
        }
        panel.Children.Add(error);Popover("New property",panel,anchor,width:280);return Task.CompletedTask;
    }
    Task ColumnSettings(string key,string name,FrameworkElement anchor){
        lastActionAnchor=anchor;var panel=new StackPanel{Spacing=4};
        if(new[]{"name","modified","type","size","priority"}.Contains(key)){
            foreach(var descending in new[]{false,true})panel.Children.Add(FlatButton(descending?"Sort descending":"Sort ascending",descending?"\uE74B":"\uE74A",async()=>{activePopover?.Hide();if(search.Text.Trim().Length>0){message.Text="Clear search to change sorting.";return;}if(key=="priority"&&route=="browse"){message.Text="Priority sorting is available in All files.";return;}sort=key;direction=descending?"desc":"asc";await PageAt(0);}));
        }
        if(key!="name")panel.Children.Add(FlatButton("Hide property","\uED1A",()=>{visibleColumns.Remove(key);BuildFileColumns();PersistTabs();activePopover?.Hide();return Task.CompletedTask;}));
        panel.Children.Add(FlatButton("Property visibility","\uE9E9",async()=>{await ClosePopover();lastActionAnchor=columnsButton;await ViewSettings();}));Popover(name,panel,anchor,240);return Task.CompletedTask;
    }
    StackPanel ChoicePicker(List<string> selected,IEnumerable<string> options,bool multiple,Func<string,Task<string>>? create,Func<Task> changed){
        var panel=new StackPanel{Spacing=8};var chips=new StackPanel{Orientation=Orientation.Horizontal,Spacing=4};var filter=new TextBox{PlaceholderText=create==null?"Find an option…":"Find an option or create one…",MaxLength=60,FontSize=12};var choices=new StackPanel{Spacing=2};var error=new TextBlock{FontSize=11,Foreground=Brush(0xe5a088),TextWrapping=TextWrapping.Wrap};var known=options.Concat(selected).Distinct(StringComparer.OrdinalIgnoreCase).ToList();
        bool busy=false;void SetBusy(bool value){busy=value;panel.IsHitTestVisible=!value;filter.IsReadOnly=value;foreach(var b in choices.Children.OfType<Button>().Concat(chips.Children.OfType<Button>()))b.IsEnabled=!value;}
        void RestoreInputFocus(){if(panel.IsLoaded)filter.Focus(FocusState.Keyboard);}
        async Task Change(string item){if(busy)return;var previous=selected.ToArray();SetBusy(true);try{if(selected.Contains(item))selected.Remove(item);else{if(!multiple)selected.Clear();selected.Add(item);}await changed();error.Text="";}catch(Exception e){selected.Clear();selected.AddRange(previous);error.Text=e.Message;}finally{SetBusy(false);Render();RestoreInputFocus();}}
        void Render(){
            chips.Children.Clear();foreach(var item in selected.ToArray()){var chip=FlatButton(item+" ×","",()=>Change(item));chip.Background=TagBrush(item);chip.FontSize=11;chips.Children.Add(chip);}
            choices.Children.Clear();var query=filter.Text.Trim();foreach(var option in known.Where(v=>v.Contains(query,StringComparison.OrdinalIgnoreCase)).Take(50)){
                var row=FlatButton(option,selected.Contains(option)?"\uE73E":"\uE739",()=>Change(option));row.HorizontalAlignment=HorizontalAlignment.Stretch;row.HorizontalContentAlignment=HorizontalAlignment.Left;choices.Children.Add(row);
            }
            if(create!=null&&query.Length>0&&!known.Any(v=>v.Equals(query,StringComparison.OrdinalIgnoreCase))){var add=FlatButton("Create “"+query+"”","\uE710",async()=>{if(busy)return;try{SetBusy(true);var saved=await create(query);known.Add(saved);filter.Text="";SetBusy(false);await Change(saved);}catch(Exception e){error.Text=e.Message;}finally{SetBusy(false);RestoreInputFocus();}});choices.Children.Add(add);}
            if(choices.Children.Count==0)choices.Children.Add(Label(query.Length>0?"No matching options":"Type to create your first option",11));
        }
        panel.Children.Add(new ScrollViewer{Content=chips,HorizontalScrollBarVisibility=ScrollBarVisibility.Auto,VerticalScrollBarVisibility=ScrollBarVisibility.Disabled,MaxHeight=40});panel.Children.Add(filter);panel.Children.Add(new ScrollViewer{Content=choices,MaxHeight=210});panel.Children.Add(error);filter.TextChanged+=(_,_)=>Render();filter.KeyDown+=(_,e)=>{if(e.Key==VirtualKey.Enter){e.Handled=true;if(choices.Children.FirstOrDefault() is Button b){var peer=new Microsoft.UI.Xaml.Automation.Peers.ButtonAutomationPeer(b);(peer.GetPattern(Microsoft.UI.Xaml.Automation.Peers.PatternInterface.Invoke) as Microsoft.UI.Xaml.Automation.Provider.IInvokeProvider)?.Invoke();}}};Render();return panel;
    }
    static SolidColorBrush TagBrush(string label){uint hash=0;foreach(var ch in label)hash=hash*31+ch;return Brush(new uint[]{0x443d29,0x334744,0x433a50,0x4c3639,0x34424e}[hash%5]);}
}

public sealed class PropertyCellConverter:Microsoft.UI.Xaml.Data.IValueConverter {
    public object Convert(object value,Type targetType,object parameter,string language){
        var key=parameter?.ToString()??"";var metadata=value as JsonNode;var item=key.StartsWith("prop:")?metadata?["properties"]?[key[5..]]:metadata?[key];
        var labels=item is JsonArray array?array.Select(v=>v?.ToString()??"").ToArray():string.IsNullOrEmpty(item?.ToString())?Array.Empty<string>():[item!.ToString()];
        var panel=new StackPanel{Orientation=Orientation.Horizontal,Spacing=3};
        foreach(var label in labels.Take(2)){uint hash=0;foreach(var ch in label)hash=hash*31+ch;var color=new uint[]{0x947a38,0x4d8070,0x806499,0x995e6b,0x577a99}[hash%5];var text=new TextBlock{Text=label,FontSize=11,MaxWidth=76,TextTrimming=TextTrimming.CharacterEllipsis};panel.Children.Add(new Border{Child=text,CornerRadius=new CornerRadius(4),Padding=new Thickness(5,2,5,2),Background=new SolidColorBrush(Windows.UI.Color.FromArgb(65,(byte)(color>>16),(byte)(color>>8),(byte)color))});}
        if(labels.Length>2)panel.Children.Add(new TextBlock{Text="+"+(labels.Length-2),FontSize=10,Opacity=.6,VerticalAlignment=VerticalAlignment.Center});
        if(labels.Length==0)panel.Children.Add(new TextBlock{Text="—",FontSize=11,Opacity=.3});
        AutomationProperties.SetName(panel,labels.Length==0?"Empty":string.Join(", ",labels));return panel;
    }
    public object ConvertBack(object value,Type targetType,object parameter,string language)=>throw new NotSupportedException();
}
