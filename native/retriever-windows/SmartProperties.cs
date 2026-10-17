using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using System.Globalization;
using System.Text.Json.Nodes;

namespace Retriever.Native;

public sealed partial class MainWindow {
    static int PropertyChoiceIndex(string[] choices,string? value)=>Math.Max(0,Array.IndexOf(choices,value)+1);
    async Task Properties(){
        var file=Selected();await LoadPropertyDefinitions();var metadata=await api.Post("workspace/inspect",new JsonObject{["scope"]="filesystem",["device"]=file.Device,["path"]=file.Path});
        var body=new StackPanel{Spacing=8};body.Children.Add(new TextBlock{Text=file.Name,FontSize=14,FontWeight=Microsoft.UI.Text.FontWeights.SemiBold,TextTrimming=TextTrimming.CharacterEllipsis});
        body.Children.Add(new TextBlock{Text=file.Path,FontSize=10,Opacity=.5,TextWrapping=TextWrapping.Wrap,MaxLines=2});
        foreach(var (key,name) in new[]{("tags","Tags"),("priority","Priority"),("status","Status"),("owner","Owner")}.Concat(propertyDefinitions.Select(d=>("prop:"+d!["id"],d["name"]!.ToString())))){
            var button=FlatButton(name,"\uE8D2",()=>Task.CompletedTask);button.HorizontalAlignment=HorizontalAlignment.Stretch;button.HorizontalContentAlignment=HorizontalAlignment.Left;
            button.Click+=async(_,_)=>{await ClosePopover();await Guard(()=>EditFileProperty(file,key,PropertyAnchor(file)));};body.Children.Add(button);
        }
        var favorite=new CheckBox{Content="Favorite",IsChecked=metadata["starred"]?.GetValue<bool>()==true,FontSize=12};favorite.Click+=async(_,_)=>await Guard(async()=>{metadata["starred"]=favorite.IsChecked==true;metadata=await SaveFileMetadata(file,metadata);});body.Children.Add(favorite);
        body.Children.Add(new Border{Height=1,Background=line,Margin=new Thickness(0,6,0,6)});body.Children.Add(Label("Comments",12));
        var comments=new StackPanel{Spacing=8};void RenderComments(JsonNode value){comments.Children.Clear();foreach(var item in value["comments"]?.AsArray()??[]){if(item==null)continue;var p=new StackPanel{Spacing=3};p.Children.Add(new TextBlock{Text=item["author"]?.ToString(),FontSize=10,Opacity=.6});p.Children.Add(new TextBlock{Text=item["body"]?.ToString(),TextWrapping=TextWrapping.Wrap,FontSize=12});comments.Children.Add(p);}if(comments.Children.Count==0)comments.Children.Add(Label("No comments yet",11));}RenderComments(metadata);
        body.Children.Add(new ScrollViewer{Content=comments,MaxHeight=140});var comment=new TextBox{PlaceholderText="Add a comment…",AcceptsReturn=true,MaxHeight=90,MaxLength=8000};body.Children.Add(comment);
        body.Children.Add(FlatButton("Post comment","\uE724",async()=>{if(string.IsNullOrWhiteSpace(comment.Text))return;var data=await api.Post("workspace/comment",new JsonObject{["scope"]="filesystem",["device"]=file.Device,["path"]=file.Path,["author"]=Environment.UserName,["body"]=comment.Text.Trim()});RenderComments(data);comment.Text="";}));
        var flyout=Popover("Properties & comments",new ScrollViewer{Content=body,MaxHeight=490},PropertyAnchor(file),340);
#if RETRIEVER_TESTING
        if(!fixtureInteractive&&Environment.GetEnvironmentVariable("RETRIEVER_PROFILE")?.StartsWith("workspace-smoke-health-offline")==true){await RenderWorkspaceTest(body,"workspace-properties.png");flyout.Hide();}
#endif
    }
    FrameworkElement PropertyAnchor(FileRow file)=>(gridMode?(ListViewBase)icons:files).ContainerFromItem(file) as FrameworkElement??(FrameworkElement?)columnsButton??search;
    async Task<JsonNode> SaveFileMetadata(FileRow file,JsonNode metadata){
        var body=metadata.DeepClone().AsObject();body["scope"]="filesystem";body["device"]=file.Device;body["path"]=file.Path;
        var result=await api.Post("workspace/metadata",body);file.SetMetadata(result);return result;
    }
    async Task EditFileProperty(FileRow file,string key,FrameworkElement? anchor=null){
        await LoadPropertyDefinitions();var metadata=await api.Post("workspace/inspect",new JsonObject{["scope"]="filesystem",["device"]=file.Device,["path"]=file.Path});var custom=key.StartsWith("prop:",StringComparison.Ordinal);var id=custom?key[5..]:key;
        var definition=custom?propertyDefinitions.FirstOrDefault(d=>d?["id"]?.ToString()==id):null;var kind=definition?["kind"]?.ToString()??(key=="tags"?"multi_select":key=="priority"?"select":"text");var title=definition?["name"]?.ToString()??char.ToUpper(key[0])+key[1..];
        var value=custom?metadata["properties"]?[id]:metadata[key];var body=new StackPanel{Spacing=9};var error=new TextBlock{FontSize=11,Foreground=Brush(0xe5a088),TextWrapping=TextWrapping.Wrap};bool saving=false,closed=false;
        async Task Save(JsonNode? next){
            if(saving)return;saving=true;
            try{var updated=metadata.DeepClone();if(custom||key=="priority"){updated["properties"]??=new JsonObject();updated["properties"]![id]=next?.DeepClone();}else updated[key]=next?.DeepClone();
                metadata=await SaveFileMetadata(file,updated);message.Text=title+" saved";
            }finally{saving=false;if(closed&&route is "browse" or "library")await Guard(RefreshCurrent);}
        }
        if(kind is "select" or "multi_select"){
            bool multi=kind=="multi_select";var selected=multi?(value as JsonArray??[]).Select(v=>v!.ToString()).ToList():string.IsNullOrEmpty(value?.ToString())?new List<string>():[value!.ToString()];
            var options=custom?(definition!["options"]?.AsArray()??[]).Select(v=>v!.ToString()).ToArray():key=="priority"?new[]{"low","medium","high","urgent"}:workspaceTags;
            Func<string,Task<string>>? create=key=="priority"?null:async text=>{if(custom){var updated=await AddPropertyOption(id,text);return updated["options"]!.AsArray().First(v=>v!.ToString().Equals(text,StringComparison.OrdinalIgnoreCase))!.ToString();}return text;};
            body.Children.Add(ChoicePicker(selected,options,multi,create,async()=>await Save(multi?new JsonArray(selected.Select(s=>(JsonNode?)JsonValue.Create(s)).ToArray()):JsonValue.Create(selected.FirstOrDefault()??""))));
            body.Children.Add(new TextBlock{Text="Changes save as you select. Click outside to close.",FontSize=10,Opacity=.55,TextWrapping=TextWrapping.Wrap});
        }else if(kind=="checkbox"){
            var check=new CheckBox{Content=title,IsChecked=value?.GetValue<bool>()==true};check.Click+=async(_,_)=>{check.IsEnabled=false;try{await Save(JsonValue.Create(check.IsChecked==true));error.Text="";}catch(Exception ex){check.IsChecked=metadata["properties"]?[id]?.GetValue<bool>()==true;error.Text=ex.Message;}finally{check.IsEnabled=true;}};body.Children.Add(check);
        }else{
            var input=new TextBox{Text=value?.ToString()??"",PlaceholderText=kind=="date"?"YYYY-MM-DD":kind=="number"?"A number":"Empty",MaxLength=kind=="text"?2000:40};body.Children.Add(input);
            body.Children.Add(FlatButton("Save","\uE73E",async()=>{try{JsonNode? next=JsonValue.Create(input.Text.Trim());if(kind=="number"){if(string.IsNullOrWhiteSpace(input.Text))next=null;else if(double.TryParse(input.Text,NumberStyles.Float,CultureInfo.InvariantCulture,out var n)&&double.IsFinite(n))next=JsonValue.Create(n);else throw new InvalidOperationException("Enter a number using a decimal point.");}await Save(next);activePopover?.Hide();}catch(Exception e){error.Text=e.Message;}}));
        }
        body.Children.Add(error);var popup=Popover(title,body,anchor??PropertyAnchor(file),300);
        popup.Closed+=async(_,_)=>{closed=true;if(!saving&&route is "browse" or "library")await Guard(RefreshCurrent);};
    }
    Task SmartFilters(){
        var rules=location["filters"]?["rules"] as JsonArray??[];string Existing(string field)=>rules.FirstOrDefault(r=>r?["field"]?.ToString()==field)?["value"]?.ToString()??"";
        var name=new TextBox{PlaceholderText="Name contains…",Text=Existing("name")};var tag=new TextBox{PlaceholderText="Tag contains…",Text=Existing("tags")};var state=new TextBox{PlaceholderText="Status is…",Text=Existing("status")};
        var kinds=new[]{"All types","pdf","docx","pptx","xlsx","txt","md","png","jpg","mp4","mkv"};var type=new ComboBox{ItemsSource=kinds,SelectedIndex=Math.Max(0,Array.IndexOf(kinds,Existing("extension"))),HorizontalAlignment=HorizontalAlignment.Stretch};
        var levels=new[]{"Any priority","low","medium","high","urgent"};var priority=new ComboBox{ItemsSource=levels,SelectedIndex=Math.Max(0,Array.IndexOf(levels,Existing("priority"))),HorizontalAlignment=HorizontalAlignment.Stretch};
        var panel=new StackPanel{Spacing=8};foreach(var item in new UIElement[]{name,type,tag,priority,state})panel.Children.Add(item);
        var actions=new StackPanel{Orientation=Orientation.Horizontal,Spacing=8};actions.Children.Add(FlatButton("Apply filters","\uE73E",async()=>{
            var next=new JsonArray();void Rule(string f,string op,string v){if(v.Trim().Length>0)next.Add(new JsonObject{["field"]=f,["operator"]=op,["value"]=v.Trim()});}Rule("name","contains",name.Text);Rule("tags","contains",tag.Text);Rule("status","is",state.Text);if(type.SelectedIndex>0)Rule("extension","is",type.SelectedItem!.ToString()!);if(priority.SelectedIndex>0)Rule("priority","is",priority.SelectedItem!.ToString()!);activePopover?.Hide();await ApplyFilters(next);
        }));actions.Children.Add(FlatButton("Clear","",async()=>{activePopover?.Hide();await ApplyFilters(new());}));panel.Children.Add(actions);Popover("Filter · all conditions",panel,filterButton,300);return Task.CompletedTask;
    }
    async Task ApplyFilters(JsonArray rules){typing?.Cancel();search.Text="";route="library";location=new JsonObject{["filters"]=new JsonObject{["match"]="all",["rules"]=rules}};currentTitle=rules.Count>0?"Filtered files":"All files";offset=0;SelectNavigation("files");await LoadFiles();}
}
