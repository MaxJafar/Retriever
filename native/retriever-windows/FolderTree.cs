using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using System.Text.Json.Nodes;

namespace Retriever.Native;

public sealed partial class MainWindow {
    sealed class FolderBranch {
        public JsonArray Entries=new();public int? Next;public bool Loaded,Loading;public string Error="";
    }
    readonly StackPanel folderTree=new();
    readonly Dictionary<string,FolderBranch> folderBranches=new();
    readonly HashSet<string> expandedFolders=new();
    readonly HashSet<string> collapsedDevices=new();
    readonly HashSet<string> folderNavKeys=new();
    string treeEndpoint="";
    string TreePath=>Path.Combine(api.StatePath,"folder-tree.json");
    void RestoreFolderTree(){
        if(treeEndpoint==api.Endpoint)return;treeEndpoint=api.Endpoint;explorerLocations.Clear();folderBranches.Clear();expandedFolders.Clear();collapsedDevices.Clear();
        try{if(File.Exists(TreePath)&&new FileInfo(TreePath).Length<128*1024){var saved=JsonNode.Parse(File.ReadAllText(TreePath));if(saved?["endpoint"]?.ToString()==api.Endpoint){utilitiesCollapsed=saved["utilities_collapsed"]?.GetValue<bool>()==true;foreach(var key in (saved["expanded"] as JsonArray??[]).Take(256))if(key!=null)expandedFolders.Add(key.ToString());foreach(var key in saved["collapsed"] as JsonArray??[])if(key!=null)collapsedDevices.Add(key.ToString());}}}catch{ /* Invalid tree state starts collapsed. */ }
    }
    void SaveFolderTree(){
        try{Directory.CreateDirectory(api.StatePath);var data=new JsonObject{["endpoint"]=api.Endpoint,["utilities_collapsed"]=utilitiesCollapsed,["expanded"]=new JsonArray(expandedFolders.Take(256).Select(v=>(JsonNode?)JsonValue.Create(v)).ToArray()),["collapsed"]=new JsonArray(collapsedDevices.Select(v=>(JsonNode?)JsonValue.Create(v)).ToArray())};File.WriteAllText(TreePath+".tmp",data.ToJsonString());File.Move(TreePath+".tmp",TreePath,true);}catch(Exception e){message.Text="Could not save folder expansion: "+e.Message;}
    }
    async Task BuildFolderTree(){
        RestoreFolderTree();
        foreach(var d in config?["devices"]?.AsArray()??[]){if(d==null||d["enabled"]?.GetValue<bool>()==false)continue;var id=d["id"]!.ToString();if(d["transport"]?.ToString()=="local"&&!explorerLocations.ContainsKey(id)){try{explorerLocations[id]=await DeviceLocations(id);}catch(Exception e){message.Text="Locations unavailable: "+e.Message;}}}
        RenderFolderTree();
    }
    void RenderFolderTree(){
        folderTree.Children.Clear();foreach(var key in folderNavKeys)navButtons.Remove(key);folderNavKeys.Clear();
        foreach(var d in config?["devices"]?.AsArray()??[]){
            if(d==null||d["enabled"]?.GetValue<bool>()==false)continue;var id=d["id"]!.ToString();var name=d["label"]?.ToString()??id;
            AddTreeRow(name,"\uE7F4","device:"+id,0,!collapsedDevices.Contains(id),true,()=>BrowseDevice(id),async()=>{if(!collapsedDevices.Add(id))collapsedDevices.Remove(id);SaveFolderTree();if(!collapsedDevices.Contains(id)&&!explorerLocations.ContainsKey(id))explorerLocations[id]=await DeviceLocations(id);RenderFolderTree();});
            if(collapsedDevices.Contains(id))continue;
            if(!explorerLocations.ContainsKey(id)&&d["transport"]?.ToString()=="ssh"){var connect=FlatButton("Connect to browse","\uE968",async()=>{explorerLocations[id]=await DeviceLocations(id);RenderFolderTree();});connect.Margin=new Thickness(22,0,4,4);folderTree.Children.Add(connect);}
            var discovered=explorerLocations.GetValueOrDefault(id)??new JsonArray();
            foreach(var entry in discovered){if(entry==null)continue;RenderFolderBranch(id,entry["path"]!.ToString(),entry["name"]?.ToString()??"Folder",entry["kind"]?.ToString()=="home"?"\uE80F":"\uEDA2",1);}
            foreach(var p in discovered.Count==0?d["roots"]?.AsArray()??[]:new JsonArray()){if(p==null||discovered.Any(v=>v?["path"]?.ToString()==p.ToString()))continue;var path=p.ToString();RenderFolderBranch(id,path,path.Replace('\\','/').TrimEnd('/').Split('/').Last(),"\uE8B7",1);}
        }
        SelectNavigation(selectedNavigation);
    }
    void AddTreeRow(string name,string glyph,string key,int depth,bool expanded,bool expandable,Func<Task> open,Func<Task> toggle){
        var row=new Grid{Margin=new Thickness(4+Math.Min(depth,12)*12,0,4,1)};row.ColumnDefinitions.Add(new(){Width=new GridLength(18)});row.ColumnDefinitions.Add(new(){Width=new GridLength(1,GridUnitType.Star)});
        var chevron=FlatButton((expanded?"Collapse ":"Expand ")+name,expanded?"\uE70D":"\uE76C",toggle,true);chevron.Padding=new Thickness(0);chevron.MinWidth=18;chevron.IsEnabled=expandable;chevron.Opacity=expandable?1:.25;row.Children.Add(chevron);
        var label=FlatButton(name,glyph,open);label.Padding=new Thickness(3,3,3,3);label.HorizontalAlignment=HorizontalAlignment.Stretch;label.HorizontalContentAlignment=HorizontalAlignment.Stretch;label.MinWidth=0;
        if(label.Content is StackPanel panel){panel.Spacing=6;if(glyph=="\uE8B7"&&panel.Children[0] is FontIcon icon){icon.Foreground=accent;panel.Children.RemoveAt(0);var native=new Image{Width=16,Height=16};native.SetBinding(Image.SourceProperty,new Microsoft.UI.Xaml.Data.Binding{Source=new FileRow(new JsonObject{["kind"]="folder",["path"]=key[(key.IndexOf(':')+1)..]}),Path=new PropertyPath("Icon")});var frame=new Grid{Width=16,Height=16};frame.Children.Add(icon);frame.Children.Add(native);panel.Children.Insert(0,frame);} if(panel.Children.Last() is TextBlock text)text.MaxWidth=Math.Max(48,152-depth*12);}
        AutomationProperties.SetName(label,name);ToolTipService.SetToolTip(label,key);Grid.SetColumn(label,1);row.Children.Add(label);folderTree.Children.Add(row);navButtons[key]=label;folderNavKeys.Add(key);
        var menu=new MenuFlyout();
        if(!key.StartsWith("device:")){var split=key.IndexOf(':');var device=key[..split];var path=key[(split+1)..];var tab=new MenuFlyoutItem{Text="Open in new tab"};tab.Click+=async(_,_)=>await Guard(async()=>{await NewTab(FolderTabState(device,path));});menu.Items.Add(tab);var refresh=new MenuFlyoutItem{Text="Refresh folders"};refresh.Click+=async(_,_)=>await Guard(()=>LoadFolderBranch(device,path,true));menu.Items.Add(refresh);}
        else {var refresh=new MenuFlyoutItem{Text="Refresh locations"};refresh.Click+=async(_,_)=>await Guard(async()=>{var id=key[7..];explorerLocations[id]=await DeviceLocations(id);RenderFolderTree();});menu.Items.Add(refresh);}
        label.ContextFlyout=menu;
    }
    void RenderFolderBranch(string device,string path,string name,string glyph,int depth){
        var key=device+":"+path;var expanded=expandedFolders.Contains(key);var branch=folderBranches.GetValueOrDefault(key);
        AddTreeRow(name,glyph,key,depth,expanded,branch?.Loaded!=true||branch.Entries.Count>0||branch.Error.Length>0,()=>Browse(device,path),async()=>{if(!expandedFolders.Add(key))expandedFolders.Remove(key);SaveFolderTree();RenderFolderTree();if(expandedFolders.Contains(key))await LoadFolderBranch(device,path);});
        if(!expanded)return;
        if(branch==null){TreeNote("Loading folders…",depth+1);DispatcherQueue.TryEnqueue(async()=>await Guard(()=>LoadFolderBranch(device,path)));return;}
        if(branch.Loading){TreeNote("Loading folders…",depth+1);return;}
        if(branch.Error.Length>0){var retry=FlatButton("Retry · "+branch.Error,"\uE72C",()=>LoadFolderBranch(device,path,true));retry.Margin=new Thickness(18+depth*12,0,4,0);ToolTipService.SetToolTip(retry,branch.Error);folderTree.Children.Add(retry);return;}
        if(depth>=16){TreeNote("Open in a tab to go deeper",depth+1);return;}
        foreach(var entry in branch.Entries){if(entry!=null)RenderFolderBranch(device,entry["path"]!.ToString(),entry["name"]!.ToString(),"\uE8B7",depth+1);}
        if(branch.Loaded&&branch.Entries.Count==0)TreeNote("No subfolders",depth+1);
        if(branch.Next!=null&&branch.Entries.Count<1000){var more=FlatButton("More folders","\uE712",()=>LoadFolderBranch(device,path,false,true));more.Margin=new Thickness(18+depth*12,0,4,0);folderTree.Children.Add(more);}else if(branch.Next!=null)TreeNote("Open in a tab for more folders",depth+1);
    }
    void TreeNote(string text,int depth){var note=Label(text,10);note.Opacity=.55;note.Margin=new Thickness(23+depth*12,4,6,5);folderTree.Children.Add(note);}
    async Task LoadFolderBranch(string device,string path,bool refresh=false,bool more=false){
        var key=device+":"+path;if(!folderBranches.TryGetValue(key,out var branch)){if(folderBranches.Count>=256){foreach(var unused in folderBranches.Where(v=>!v.Value.Loading&&!expandedFolders.Contains(v.Key)).Select(v=>v.Key).ToArray())folderBranches.Remove(unused);if(folderBranches.Count>=256){message.Text="Collapse unused folders before expanding more.";expandedFolders.Remove(key);RenderFolderTree();return;}}folderBranches[key]=branch=new();}
        if(branch.Loading||branch.Loaded&&!refresh&&!more)return;branch.Loading=true;branch.Error="";RenderFolderTree();
        try{var result=await api.Post("explorer/children",new JsonObject{["device"]=device,["path"]=path,["offset"]=more?branch.Next??0:0});if(!more)branch.Entries.Clear();foreach(var item in result["entries"]?.AsArray()??[])if(item!=null)branch.Entries.Add(item.DeepClone());branch.Next=result["next_offset"]?.GetValue<int>();branch.Loaded=true;}catch(Exception e){branch.Error=e.Message;}finally{branch.Loading=false;RenderFolderTree();}
    }
}
