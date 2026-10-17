using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using System.Text.Json.Nodes;
using Windows.System;

namespace Retriever.Native;

public sealed partial class MainWindow {
    sealed class ExplorerTab {
        public string Id=Guid.NewGuid().ToString("N");
        public bool Pinned;
        public JsonObject State=new();
    }
    // Internal reordering uses the native strip; exporting tabs as data needs a
    // separate transfer protocol and must not intercept the reorder gesture.
    readonly TabView tabStrip=new(){IsAddTabButtonVisible=true,CanDragTabs=false,CanReorderTabs=true,AllowDropTabs=true,TabWidthMode=TabViewWidthMode.SizeToContent};
    readonly List<ExplorerTab> closedTabs=[];
    TabViewItem? activeTab;
    bool tabsReady,changingTabs,draggingTabs;
    string TabsPath=>Path.Combine(api.StatePath,"explorer-tabs.json");
    static T TabValue<T>(JsonNode? value,T fallback)=>value is JsonValue scalar&&scalar.TryGetValue<T>(out var result)?result:fallback;
    static JsonObject EmptyTabState()=>new(){["route"]="library",["title"]="All files",["nav"]="files",["location"]=new JsonObject(),["query"]="",["sort"]="name",["direction"]="asc"};
    UIElement BuildTabs(){
        // Root-level shortcuts apply everywhere; their automatic tooltip would
        // otherwise appear over unrelated file rows and dashboard controls.
        root.KeyboardAcceleratorPlacementMode=KeyboardAcceleratorPlacementMode.Hidden;
        tabStrip.Margin=new Thickness(-6,0,0,0);
        tabStrip.AddTabButtonClick+=async(_,_)=>await Guard(()=>NewTab());
        tabStrip.TabCloseRequested+=async(_,e)=>await Guard(()=>CloseTab(e.Tab));
        tabStrip.SelectionChanged+=async(_,_)=>{if(tabsReady&&!changingTabs&&!draggingTabs&&tabStrip.SelectedItem is TabViewItem tab)await Guard(()=>ActivateTab(tab));};
        tabStrip.TabItemsChanged+=(_,_)=>{if(tabsReady&&!changingTabs&&!draggingTabs)DispatcherQueue.TryEnqueue(()=>PersistTabs());};
        tabStrip.TabDragStarting+=(_,_)=>{CaptureTab();draggingTabs=true;};
        tabStrip.TabDragCompleted+=async(_,_)=>{draggingTabs=false;if(tabStrip.SelectedItem is TabViewItem tab)await Guard(()=>ActivateTab(tab));PersistTabs();};
        AddTabItem(new ExplorerTab{State=EmptyTabState()});
        activeTab=(TabViewItem)tabStrip.TabItems[0];tabStrip.SelectedItem=activeTab;
        void Key(VirtualKey key,VirtualKeyModifiers modifiers,Func<Task> action){var k=new KeyboardAccelerator{Key=key,Modifiers=modifiers};k.Invoked+=async(_,e)=>{if(dialogVisible)return;e.Handled=true;await Guard(action);};root.KeyboardAccelerators.Add(k);}
        Key(VirtualKey.T,VirtualKeyModifiers.Control,()=>NewTab());
        Key(VirtualKey.W,VirtualKeyModifiers.Control,()=>activeTab==null?Task.CompletedTask:CloseTab(activeTab));
        Key(VirtualKey.T,VirtualKeyModifiers.Control|VirtualKeyModifiers.Shift,ReopenTab);
        // TextBox handles Tab before a bubbling accelerator can switch tabs.
        root.PreviewKeyDown+=async(_,e)=>{
            if(e.Handled)return;
            static bool Down(VirtualKey key)=>(Microsoft.UI.Input.InputKeyboardSource.GetKeyStateForCurrentThread(key)&Windows.UI.Core.CoreVirtualKeyStates.Down)!=0;
            var step=TabCycleStep(e.Key,Down(VirtualKey.Control),Down(VirtualKey.Shift),Down(VirtualKey.Menu));
            if(step==0)return;e.Handled=true;await Guard(()=>CycleTab(step));
        };
        return tabStrip;
    }
    int TabCycleStep(VirtualKey key,bool control,bool shift,bool alt)=>tabsReady&&!dialogVisible&&tabStrip.TabItems.Count>0&&key==VirtualKey.Tab&&control&&!alt?(shift?-1:1):0;
    TabViewItem AddTabItem(ExplorerTab state){
        var item=new TabViewItem{Tag=state,Header="All files",IsClosable=!state.Pinned,MinWidth=110,MaxWidth=220};
        var menu=new MenuFlyout();
        foreach(var (label,action) in new (string,Func<Task>)[]{
            ("Duplicate tab",async()=>{if(item==activeTab)CaptureTab();await NewTab(state.State);}),
            ("Pin / unpin tab",()=>{state.Pinned=!state.Pinned;UpdateTabHeaders();PersistTabs();return Task.CompletedTask;}),
            ("Move tab left",()=>MoveTab(item,-1)),("Move tab right",()=>MoveTab(item,1)),
            ("Close tab",()=>CloseTab(item)),("Reopen closed tab",ReopenTab)}){
            var entry=new MenuFlyoutItem{Text=label};entry.Click+=async(_,_)=>await Guard(action);menu.Items.Add(entry);
        }
        item.ContextFlyout=menu;tabStrip.TabItems.Add(item);UpdateTabHeaders();return item;
    }
    JsonArray HistoryJson(IEnumerable<(string Route,JsonObject Location,string Title,string Nav,string Query)> entries)=>new(entries.Take(20).Select(v=>(JsonNode)new JsonObject{["route"]=v.Route,["location"]=v.Location.DeepClone(),["title"]=v.Title,["nav"]=v.Nav,["query"]=v.Query}).ToArray());
    void CaptureTab(){
        if(activeTab?.Tag is not ExplorerTab tab)return;
        tab.State=new JsonObject{["route"]=route,["location"]=location.DeepClone(),["title"]=currentTitle,["nav"]=selectedNavigation,["query"]=search.Text,["sort"]=sort,["direction"]=direction,["offset"]=offset,["grid"]=gridMode,
            ["sum_lengths"]=sumLengths,["search_mode"]=searchMode,
            ["columns"]=new JsonArray(visibleColumns.Select(c=>(JsonNode?)JsonValue.Create(c)).ToArray()),
            ["selection"]=new JsonArray((gridMode?(ListViewBase)icons:files).SelectedItems.Cast<FileRow>().Select(f=>(JsonNode?)JsonValue.Create(f.Device+":"+f.Path)).ToArray()),
            ["back"]=HistoryJson(backStack),["forward"]=HistoryJson(forwardStack)};
    }
    void RestoreHistory(Stack<(string Route,JsonObject Location,string Title,string Nav,string Query)> stack,JsonNode? value){
        stack.Clear();foreach(var v in (value as JsonArray??[]).Take(20).Reverse()){if(v is not JsonObject)continue;stack.Push((v["route"]?.ToString() is "browse" or "disk" or "health" or "assistant" or "burner"?v["route"]!.ToString():"library",v["location"]?.DeepClone() as JsonObject??new(),v["title"]?.ToString()??"All files",v["nav"]?.ToString()??"files",v["query"]?.ToString()??""));}
    }
    void UpdateTabHeaders(){foreach(var item in tabStrip.TabItems.OfType<TabViewItem>()){if(item.Tag is not ExplorerTab tab)continue;var q=tab.State["query"]?.ToString();var title=string.IsNullOrWhiteSpace(q)?tab.State["title"]?.ToString()??"All files":"Search: "+q;title=title.Replace('\\','/').TrimEnd('/').Split('/').Last();item.Header=(tab.Pinned?"◆ ":"")+title;item.IsClosable=!tab.Pinned;ToolTipService.SetToolTip(item,title);}}
    void PersistTabs(){
        if(!tabsReady||changingTabs||draggingTabs||quick)return;CaptureTab();UpdateTabHeaders();
        try{var tabs=new JsonArray(tabStrip.TabItems.OfType<TabViewItem>().Select(t=>{var v=(ExplorerTab)t.Tag;return (JsonNode)new JsonObject{["id"]=v.Id,["pinned"]=v.Pinned,["state"]=v.State.DeepClone()};}).ToArray());
            var data=new JsonObject{["version"]=1,["endpoint"]=api.Endpoint,["active"]=(activeTab?.Tag as ExplorerTab)?.Id,["tabs"]=tabs};
            Directory.CreateDirectory(api.StatePath);var pending=TabsPath+".tmp";File.WriteAllText(pending,data.ToJsonString());File.Move(pending,TabsPath,true);
        }catch(Exception ex){message.Text="Could not save tabs: "+ex.Message;}
    }
    async Task RestoreTabs(){
        changingTabs=true;
        try{if(File.Exists(TabsPath)&&new FileInfo(TabsPath).Length<=2*1024*1024){var saved=JsonNode.Parse(await File.ReadAllTextAsync(TabsPath));if(saved?["version"]?.GetValue<int>()==1&&saved["endpoint"]?.ToString()==api.Endpoint&&saved["tabs"] is JsonArray items&&items.Count>0){
            tabStrip.TabItems.Clear();foreach(var value in items.Take(24)){if(value?["state"] is not JsonObject s)continue;AddTabItem(new ExplorerTab{Id=value["id"]?.ToString()??Guid.NewGuid().ToString("N"),Pinned=value["pinned"]?.GetValue<bool>()==true,State=s.DeepClone().AsObject()});}
            tabStrip.SelectedItem=tabStrip.TabItems.OfType<TabViewItem>().FirstOrDefault(t=>((ExplorerTab)t.Tag).Id==saved["active"]?.ToString())??tabStrip.TabItems.FirstOrDefault();
        }}}catch(Exception){tabStrip.TabItems.Clear();message.Text="The previous tab session could not be read. Starting with All files.";}
        finally{changingTabs=false;tabsReady=true;}
        if(tabStrip.TabItems.Count==0)AddTabItem(new ExplorerTab{State=EmptyTabState()});
        var selected=tabStrip.SelectedItem as TabViewItem??(TabViewItem)tabStrip.TabItems[0];activeTab=null;await ActivateTab(selected);
    }
    async Task ActivateTab(TabViewItem item){
        if(item==activeTab)return;StopBurnerPolling();CaptureTab();typing?.Cancel();Interlocked.Increment(ref querySequence);activeTab=item;
        changingTabs=true;tabStrip.SelectedItem=item;changingTabs=false;
        var s=((ExplorerTab)item.Tag).State.DeepClone().AsObject();route=s["route"]?.ToString() is "browse" or "disk" or "health" or "assistant" or "burner"?s["route"]!.ToString():"library";location=s["location"] as JsonObject??new();currentTitle=s["title"]?.ToString()??"All files";SelectNavigation(s["nav"]?.ToString()??"files");
        sort=s["sort"]?.ToString()??"name";if(!new[]{"name","modified","size","type","priority","owner"}.Contains(sort))sort="name";direction=s["direction"]?.ToString()=="desc"?"desc":"asc";offset=Math.Clamp(TabValue(s["offset"],0),0,1_000_000);gridMode=TabValue(s["grid"],false);
        sumLengths=TabValue(s["sum_lengths"],true);searchMode=s["search_mode"]?.ToString() is "exact" or "semantic"?s["search_mode"]!.ToString():"hybrid";
        visibleColumns.Clear();foreach(var c in (s["columns"] as JsonArray??new JsonArray("modified","type","size","device")))if(c!=null)visibleColumns.Add(c.ToString());BuildFileColumns();
        RestoreHistory(backStack,s["back"]);RestoreHistory(forwardStack,s["forward"]);rows.Clear();search.Text=s["query"]?.ToString()??"";
        try{if(search.Text.Length>0)await Search(true);else await LoadFiles();}finally{if(activeTab==item){var selected=(s["selection"] as JsonArray??[]).Select(v=>v?.ToString()).ToHashSet();var view=gridMode?(ListViewBase)icons:files;foreach(var row in rows.Where(r=>selected.Contains(r.Device+":"+r.Path)))view.SelectedItems.Add(row);PersistTabs();DispatcherQueue.TryEnqueue(()=>{if(activeTab==item)item.StartBringIntoView(new BringIntoViewOptions{AnimationDesired=new Windows.UI.ViewManagement.UISettings().AnimationsEnabled});});}}
    }
    async Task NewTab(JsonObject? state=null){if(tabStrip.TabItems.Count>=24){message.Text="Close a tab before opening another (24-tab limit).";return;}CaptureTab();changingTabs=true;var item=AddTabItem(new ExplorerTab{State=state?.DeepClone().AsObject()??EmptyTabState()});changingTabs=false;await ActivateTab(item);}
    string DestinationKey(JsonObject state){
        var kind=state["route"]?.ToString()??"library";
        if(!string.IsNullOrWhiteSpace(state["query"]?.ToString()))return "";
        if(kind is "health" or "disk" or "assistant" or "burner")return kind;
        if(kind!="browse")return "";
        var device=state["location"]?["device"]?.ToString()??"";var path=state["location"]?["path"]?.ToString()??"";
        var windows=config?["devices"]?.AsArray().FirstOrDefault(d=>d?["id"]?.ToString()==device)?["platform"]?.ToString()=="windows";
        if(windows)path=path.Replace('\\','/').ToUpperInvariant();path=path.TrimEnd('/');
        return "browse\n"+device+"\n"+path;
    }
    async Task<bool> ReuseDestination(JsonObject state){
        CaptureTab();var key=DestinationKey(state);if(key.Length==0)return false;
        var match=tabStrip.TabItems.OfType<TabViewItem>().OrderBy(t=>t==activeTab?0:1).FirstOrDefault(t=>t.Tag is ExplorerTab tab&&DestinationKey(tab.State)==key);
        if(match==null)return false;await ActivateTab(match);return true;
    }
    async Task NavigateDestination(JsonObject state,bool newTab=false){
        CloseSearchExpansion();if(newTab){await NewTab(state);return;}if(await ReuseDestination(state))return;
        if(activeTab?.Tag is not ExplorerTab tab){await NewTab(state);return;}
        var item=activeTab;backStack.Push((route,location.DeepClone().AsObject(),currentTitle,selectedNavigation,search.Text));
        state["back"]=HistoryJson(backStack);state["forward"]=new JsonArray();
        tab.State=state.DeepClone().AsObject();activeTab=null;await ActivateTab(item);
    }
    JsonObject FolderTabState(string device,string path){var state=EmptyTabState();state["route"]="browse";state["location"]=new JsonObject{["device"]=device,["path"]=path,["filesystem"]=true};state["title"]=path;state["nav"]=device+":"+path;return state;}
    async Task CloseTab(TabViewItem item){if(item.Tag is not ExplorerTab tab||tab.Pinned)return;if(item==activeTab)CaptureTab();closedTabs.Add(new ExplorerTab{State=tab.State.DeepClone().AsObject()});if(closedTabs.Count>10)closedTabs.RemoveAt(0);var index=tabStrip.TabItems.IndexOf(item);changingTabs=true;tabStrip.TabItems.Remove(item);if(tabStrip.TabItems.Count==0)AddTabItem(new ExplorerTab{State=EmptyTabState()});changingTabs=false;if(item==activeTab)await ActivateTab((TabViewItem)tabStrip.TabItems[Math.Clamp(index,0,tabStrip.TabItems.Count-1)]);PersistTabs();}
    Task ReopenTab(){if(closedTabs.Count==0)return Task.CompletedTask;if(tabStrip.TabItems.Count>=24){message.Text="Close a tab before reopening another (24-tab limit).";return Task.CompletedTask;}var tab=closedTabs[^1];closedTabs.RemoveAt(closedTabs.Count-1);return NewTab(tab.State);}
    Task CycleTab(int delta){if(tabStrip.TabItems.Count==0)return Task.CompletedTask;var i=tabStrip.TabItems.IndexOf(activeTab!);return ActivateTab((TabViewItem)tabStrip.TabItems[(i+delta+tabStrip.TabItems.Count)%tabStrip.TabItems.Count]);}
    Task MoveTab(TabViewItem item,int delta){var i=tabStrip.TabItems.IndexOf(item);var next=Math.Clamp(i+delta,0,tabStrip.TabItems.Count-1);changingTabs=true;tabStrip.TabItems.Remove(item);tabStrip.TabItems.Insert(next,item);tabStrip.SelectedItem=activeTab;changingTabs=false;PersistTabs();return Task.CompletedTask;}
    async Task OpenFolderTab(){var file=Selected();await NewTab(FolderTabState(file.Device,file.Kind=="folder"?file.Path:file.ParentPath));if(file.Kind!="folder")await RevealInRetriever(file);}
}
