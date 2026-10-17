using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Markup;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Microsoft.UI.Windowing;
using System.Collections.ObjectModel;
using System.Text.Json.Nodes;
using Windows.System;
using Windows.Graphics;

namespace Retriever.Native;
public sealed partial class MainWindow:Window {
    DispatcherTimer? crawlTimer;
    DesktopIntegration? desktop;QuickSearchWindow? quickWindow;bool quitting;
    readonly TaskCompletionSource<bool> workspaceReady=new(TaskCreationOptions.RunContinuationsAsynchronously);
    readonly TaskCompletionSource<bool> coordinatorReady=new(TaskCreationOptions.RunContinuationsAsynchronously);
    readonly Api api=new();readonly Grid root=new();readonly AutoSuggestBox search=new(){PlaceholderText="Search files, words, or ideas…",MinHeight=48,HorizontalAlignment=HorizontalAlignment.Stretch};
    readonly ListView files=new(){SelectionMode=ListViewSelectionMode.Extended,IsItemClickEnabled=true};readonly ObservableCollection<FileRow> rows=[];
    readonly GridView icons=new(){SelectionMode=ListViewSelectionMode.Extended,IsItemClickEnabled=true};
    readonly TextBlock message=new(){Text="Connecting to Retriever…",TextWrapping=TextWrapping.Wrap,Margin=new Thickness(16,8,16,8)};
    readonly TextBlock breadcrumb=new(){Text="All indexed files",FontSize=15,VerticalAlignment=VerticalAlignment.Center};
    readonly Grid page=new();
    readonly Grid paging=new();
    readonly TextBlock pageLabel=new(){VerticalAlignment=VerticalAlignment.Center,Opacity=.7};
    Button? previousPage,nextPage;int? nextOffset;int offset,total;bool gridMode;bool dialogVisible;string sort="name",direction="asc";
    readonly Stack<(string Route,JsonObject Location,string Title,string Nav,string Query)> backStack=new();
    JsonNode? config;JsonNode? status;string revision="";string route="library";JsonObject location=new();long querySequence;CancellationTokenSource? typing;readonly bool quick;
    public MainWindow(bool quick=false){
        this.quick=quick;Title=quick?"Retriever Quick Search":"Retriever";SystemBackdrop=quick?new DesktopAcrylicBackdrop():new MicaBackdrop();
        AppWindow.Resize(new SizeInt32(quick?820:1280,quick?560:860));
        BuildShell();SetupFileSelection();
        files.ItemsSource=rows;files.ContainerContentChanging+=(_,e)=>{if(e.Item is FileRow row)Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(e.ItemContainer,row.Name);var container=e.ItemContainer;DispatcherQueue.TryEnqueue(()=>{WirePropertyCells(container);WireSelectionCells(container);});};
        files.DoubleTapped+=async(_,e)=>{if(!IsPropertyCell(e.OriginalSource))await Guard(OpenFile);};

        files.KeyDown+=async(_,e)=>{if(IsPropertyCell(e.OriginalSource))return;if(e.Key==VirtualKey.Space){e.Handled=true;await Guard(Preview);}if(e.Key==VirtualKey.Enter){e.Handled=true;await Guard(OpenFile);}};
        icons.ItemsSource=rows;icons.ItemTemplate=(DataTemplate)XamlReader.Load("<DataTemplate xmlns='http://schemas.microsoft.com/winfx/2006/xaml/presentation'><StackPanel Width='150' Padding='10' Spacing='8'><Grid Height='58'><FontIcon Glyph='{Binding Glyph}' FontSize='48' Foreground='#D5A62C'/><Image Source='{Binding Icon}' Width='52' Height='52'/></Grid><TextBlock Text='{Binding Name}' MaxLines='2' TextWrapping='Wrap' TextTrimming='CharacterEllipsis' TextAlignment='Center'/><TextBlock Text='{Binding Size}' FontSize='11' Opacity='.6' TextAlignment='Center'/></StackPanel></DataTemplate>");
        icons.DoubleTapped+=async(_,_)=>await Guard(OpenFile);icons.KeyDown+=async(_,e)=>{if(e.Key==VirtualKey.Space){e.Handled=true;await Guard(Preview);}if(e.Key==VirtualKey.Enter){e.Handled=true;await Guard(OpenFile);}};
        foreach(var view in new ListViewBase[]{files,icons}){var menu=new MenuFlyout();foreach(var entry in new (string,Func<Task>)[]{("Open",OpenFile),("Quick look",Preview),("Properties & comments",Properties),("Open folder in new tab",OpenFolderTab),("Rename",Rename),("Copy files",CopyFiles),("Paste files",PasteFiles)}){var item=new MenuFlyoutItem{Text=entry.Item1};var action=entry.Item2;item.Click+=async(_,_)=>await Guard(action);menu.Items.Add(item);}view.ContextFlyout=menu;}
        foreach(var view in new ListViewBase[]{files,icons})view.AddHandler(UIElement.RightTappedEvent,new RightTappedEventHandler((_,e)=>{
            for(var source=e.OriginalSource as DependencyObject;source!=null&&source!=view;source=VisualTreeHelper.GetParent(source)){
                if(source is Microsoft.UI.Xaml.Controls.Primitives.SelectorItem {Content:FileRow row}){if(!view.SelectedItems.Contains(row))view.SelectedItem=row;break;}
            }
        }),true);
        search.TextChanged+=(_,e)=>{if(e.Reason==AutoSuggestionBoxTextChangeReason.UserInput)QueueSearch();};search.QuerySubmitted+=async(_,_)=>{typing?.Cancel();await Guard(()=>Search(true));};
        var focus=new KeyboardAccelerator{Key=VirtualKey.K,Modifiers=VirtualKeyModifiers.Control};focus.Invoked+=(_,e)=>{if(dialogVisible)return;FocusSearch(FocusState.Keyboard);OpenSearchExpansion();e.Handled=true;};root.KeyboardAccelerators.Add(focus);
        root.PreviewKeyDown+=async(_,e)=>{if(e.Handled||e.Key!=VirtualKey.Escape||dialogVisible)return;e.Handled=true;if(searchExpanded){CloseSearchExpansion();return;}typing?.Cancel();Interlocked.Increment(ref querySequence);if(search.Text.Length>0){search.Text="";await Guard(EmptySearch);}else if(quick)AppWindow.Hide();};
        root.Loaded+=async(_,_)=>await Guard(async()=>{InitializeUpdates();if(!quick&&desktop==null)desktop=new DesktopIntegration(WinRT.Interop.WindowNative.GetWindowHandle(this),ToggleQuick,RestoreMain,Quit,async()=>{RestoreMain();await coordinatorReady.Task;await Guard(OpenDriveHealth);});try {await api.StartAsync();await RefreshConfig();coordinatorReady.TrySetResult(true);} catch {coordinatorReady.TrySetResult(false);workspaceReady.TrySetResult(false);throw;}if(quick)await EmptySearch();else await RestoreTabs();workspaceReady.TrySetResult(true);if(message.Text=="Connecting to Retriever…")message.Text="Ready";await MaybeShowExplorerWelcome();DispatcherQueue.TryEnqueue(()=>FocusSearch(FocusState.Keyboard));if(desktop is {UsesFallback:true})message.Text="Ctrl+Alt+Space is in use. Quick Search is available with Ctrl+Alt+R or the tray icon.";else if(desktop is {HotkeyRegistered:false})message.Text="Quick Search shortcuts are in use. Use the tray icon.";await WorkspaceTestsIfRequested();});
        AppWindow.Closing+=(_,e)=>{CloseSearchExpansion();PersistTabs();if(!quitting&&!quick&&desktop is {TrayAvailable:true}){e.Cancel=true;AppWindow.Hide();}};
        Closed+=(_,_)=>{if(!quick){var overlay=quickWindow;quickWindow=null;overlay?.Shutdown();}updateTimer?.Stop();updater?.Cancel();crawlTimer?.Stop();crawlTimer=null;StopDiskPolling();StopHealthPolling();StopBurnerPolling();typing?.Cancel();desktop?.Dispose();api.Dispose();};
    }
    void PrepareQuick(){
        if(quitting)return;
        quickWindow ??= new QuickSearchWindow(api,()=>coordinatorReady.Task,()=>status?["local_device"]?.ToString()??"",DeviceName,RestoreMain,async file=>{RestoreMain();await Guard(()=>RevealInRetriever(file));});
    }
    static bool SameFilePath(string a,string b)=>string.Equals(a,b,a.StartsWith('/')?StringComparison.Ordinal:StringComparison.OrdinalIgnoreCase);
    bool RevealLoadedFile(string path){
        var row=rows.FirstOrDefault(r=>SameFilePath(r.Path,path));
        if(row==null)return false;
        var view=gridMode?(ListViewBase)icons:files;view.SelectedItem=row;
        DispatcherQueue.TryEnqueue(()=>{if(rows.Contains(row)){view.UpdateLayout();view.ScrollIntoView(row);}});
        return true;
    }
    async Task RevealInRetriever(FileRow file){
        await Browse(file.Device,file.Kind=="folder"?file.Path:file.ParentPath);
        if(file.Kind=="folder"||RevealLoadedFile(file.Path))return;
        var sequence=querySequence;
        var at=offset==0?nextOffset:0;
        while(at is int pageOffset){
            if(sequence!=querySequence)return;
            message.Text="Locating "+file.Name+"…";
            var body=location.DeepClone().AsObject();body["offset"]=pageOffset;body["sort"]=sort;body["direction"]=direction;
            var reply=await api.Post("explorer/browse",body);
            if(sequence!=querySequence)return;
            if((reply["entries"]?.AsArray()??[]).Any(item=>SameFilePath(item?["path"]?.ToString()??"",file.Path))){
                offset=pageOffset;await LoadFiles();
                if(querySequence==sequence+1&&!RevealLoadedFile(file.Path))message.Text="The file changed while its folder was loading.";
                return;
            }
            var next=reply["next_offset"]?.GetValue<int>();
            if(next is int value&&value<=pageOffset)throw new InvalidOperationException("The folder returned an invalid page.");
            at=next;
        }
        if(sequence==querySequence)message.Text="The file is no longer present in this folder.";
    }
    void ToggleQuick(){PrepareQuick();quickWindow?.Toggle();}
    void FocusSearch(FocusState state){
        search.ApplyTemplate();
        TextBox? Input(DependencyObject parent){for(int i=0;i<VisualTreeHelper.GetChildrenCount(parent);i++){var child=VisualTreeHelper.GetChild(parent,i);if(child is TextBox text)return text;var nested=Input(child);if(nested!=null)return nested;}return null;}
        if(Input(search) is TextBox input)input.Focus(state);else search.Focus(state);
    }
    public async void OpenQuickWhenReady(){await coordinatorReady.Task;AppWindow.Hide();ToggleQuick();}
    void RestoreMain(){if(quitting)return;AppWindow.Show();Activate();}
    void Quit(){quitting=true;Close();}
    Button Button(string text,Func<Task> action){var b=new Button{Content=text};b.Click+=async(_,_)=>{lastActionAnchor=b;await Guard(action);};return b;}
    static bool IsPropertyCell(object source){for(var node=source as DependencyObject;node!=null;node=VisualTreeHelper.GetParent(node))if(node is CheckBox {Tag:"select-file"})return true;else if(node is Button {Tag:string tag}&&tag.StartsWith("cell:"))return true;return false;}
    AppBarButton Command(string text,Symbol icon,Func<Task> action){var button=new AppBarButton{Label=text,Icon=new SymbolIcon(icon)};ToolTipService.SetToolTip(button,text);button.Click+=async(_,_)=>await Guard(action);return button;}
    async Task Guard(Func<Task> action){try{await action();}catch(OperationCanceledException){}catch(Exception ex){message.Text=ex.Message;}}
    async Task RefreshConfig(){status=await api.Get("status");var reply=await api.Get("config");config=reply["config"]!;revision=reply["revision"]!.ToString();await RefreshSidebar();await LoadPropertyDefinitions();BuildFileColumns();}
    void ShowList(){
        StopBurnerPolling();if(burnerPanel!=null)page.Children.Remove(burnerPanel);
        crawlTimer?.Stop();crawlTimer=null;StopDiskPolling();StopHealthPolling();
        if(assistantPanel!=null)page.Children.Remove(assistantPanel);
        if(diskPanel!=null)page.Children.Remove(diskPanel);
        if(healthPanel!=null)page.Children.Remove(healthPanel);
        foreach(var child in explorerChrome.Children.Skip(1))child.Visibility=Visibility.Visible;
        explorerViews.Visibility=Visibility.Visible;
        // Keep chrome attached across searches and tab switches, preserving focus
        // and the native tab strip's pointer capture during drag/reorder.
        if(!page.Children.Contains(explorerChrome)){page.Children.Clear();page.Children.Add(explorerChrome);}
        FrameworkElement view=gridMode?icons:tableScroll;FrameworkElement inactive=gridMode?tableScroll:icons;
        page.Children.Remove(inactive);
        if(!page.Children.Contains(view)){Grid.SetRow(view,1);page.Children.Add(view);}
        if(!page.Children.Contains(emptyLabel)){Grid.SetRow(emptyLabel,1);page.Children.Add(emptyLabel);}
        if(!page.Children.Contains(paging)){Grid.SetRow(paging,2);page.Children.Add(paging);}
        UpdateChrome();
    }
    async Task Navigate(string name){
        SelectNavigation(name);currentTitle=name=="favorites"?"Favorites":"All files";
        if(name!="search"){typing?.Cancel();Interlocked.Increment(ref querySequence);search.Text="";}
        if(name is "files" or "favorites"){route="library";offset=0;location=new JsonObject();if(name=="favorites")location["filters"]=new JsonObject{["match"]="all",["rules"]=new JsonArray(new JsonObject{["field"]="starred",["operator"]="is",["value"]=true})};ShowList();await LoadFiles();}
        else if(name=="search"){if(route is "disk" or "health" or "assistant" or "burner"){route="library";location=new();}ShowList();FocusSearch(FocusState.Keyboard);if(search.Text.Length>0)await Search(true);}
        else if(name=="clients")await Devices();
        else if(name=="crawl")await Crawling();
        else if(name=="activity"){var w=await api.Get("workspace");ShowCards("Activity",w["history"]?.AsArray()??[],v=>$"{v?["operation"]} · {v?["state"]}");}
        else {var w=await api.Get("workspace");ShowCards("Folder sync",w["sync_pairs"]?.AsArray()??[],v=>v?["name"]?.ToString()??"");message.Text="Imported sync baselines are retained. Mutations remain disabled until cross-device parity passes.";}
    }
    async Task LoadFiles(){if(route=="burner"){await ShowDiskBurner();return;}if(route=="assistant"){await ShowDiskAssistant();return;}if(route=="health"){await ShowDriveHealth();return;}if(route=="disk"){await ShowDiskSpace();return;}typing?.Cancel();var sequence=Interlocked.Increment(ref querySequence);search.Text="";ShowList();emptyLabel.Visibility=Visibility.Collapsed;message.Text="Loading files…";previousPage!.IsEnabled=false;nextPage!.IsEnabled=false;var body=location.DeepClone().AsObject();body["offset"]=offset;body["sort"]=sort;body["direction"]=direction;var reply=await api.Post(route=="browse"&&location["filesystem"]?.GetValue<bool>()==true?"explorer/browse":"workspace/"+route,body);if(sequence!=querySequence)return;rows.Clear();foreach(var item in reply["entries"]?.AsArray()??[])if(item!=null)rows.Add(Row(item));total=reply["total"]?.GetValue<int>()??rows.Count;nextOffset=reply["next_offset"]?.GetValue<int>();UpdatePager();message.Text=$"{total} items · {(reply["cached"]?.ToString()=="true"?"Indexed catalogue":"Live folder")}";currentTitle=location["path"]?.ToString().Replace('\\','/').TrimEnd('/').Split('/').LastOrDefault()??currentTitle;UpdateChrome();if(visibleColumns.Contains("duration"))_ = ReadVisibleLengths();}
    async Task Browse(string device,string path){if(await ReuseDestination(FolderTabState(device,path)))return;backStack.Push((route,location.DeepClone().AsObject(),currentTitle,selectedNavigation,search.Text));forwardStack.Clear();SelectNavigation(device+":"+path);search.Text="";route="browse";if(sort=="priority")sort="name";offset=0;location=new JsonObject{["device"]=device,["path"]=path,["filesystem"]=true};await LoadFiles();}
    async Task Back(){if(backStack.TryPop(out var previous)){forwardStack.Push((route,location.DeepClone().AsObject(),currentTitle,selectedNavigation,search.Text));route=previous.Route;location=previous.Location;currentTitle=previous.Title;SelectNavigation(previous.Nav);offset=0;search.Text=previous.Query;if(previous.Query.Length>0)await Search(true);else await LoadFiles();}}
    async Task Up(){var path=location["path"]?.ToString();var device=location["device"]?.ToString();if(path==null||device==null)return;string Normalize(string s)=>s.Replace('\\','/').TrimEnd('/');var roots=config?["devices"]?.AsArray().FirstOrDefault(d=>d?["id"]?.ToString()==device)?["roots"]?.AsArray();if(location["filesystem"]?.GetValue<bool>()!=true&&roots?.Any(r=>string.Equals(Normalize(r!.ToString()),Normalize(path),StringComparison.OrdinalIgnoreCase))==true)return;var trimmed=path.TrimEnd('/','\\');var index=trimmed.LastIndexOfAny(['/','\\']);if(index<0)return;var parent=trimmed[..(index==2&&trimmed[1]==':'?3:index==0?1:index)];await Browse(device,parent);}
    void UpdatePager(){previousPage!.IsEnabled=offset>0;nextPage!.IsEnabled=nextOffset!=null;pageLabel.Text=$"{total:N0} items · {(total==0?0:offset+1)}–{Math.Min(offset+rows.Count,total)}"+SelectionSummary();}
    async Task PageAt(int at){offset=Math.Max(0,at);await LoadFiles();}
    Task ToggleView(){var selected=(gridMode?(ListViewBase)icons:files).SelectedItems.Cast<FileRow>().ToArray();gridMode=!gridMode;ShowList();var active=gridMode?(ListViewBase)icons:files;active.SelectedItems.Clear();foreach(var row in selected)active.SelectedItems.Add(row);return Task.CompletedTask;}
    async void QueueSearch(){typing?.Cancel();typing=new();var cancel=typing.Token;try{await Task.Delay(40,cancel);if(!await Search(false,cancel)){await Task.Delay(420,cancel);await Search(true,cancel);}}catch(OperationCanceledException){}catch(Exception e){if(!cancel.IsCancellationRequested)message.Text=e.Message;}}
    async Task<bool> Search(bool full,CancellationToken cancel=default){var sequence=Interlocked.Increment(ref querySequence);var query=search.Text.Trim();if(query.Length==0){await EmptySearch();return true;}if(route is "disk" or "health" or "assistant" or "burner"){route="library";location=new();currentTitle="Search results";}ShowList();var reply=await api.Get($"search?q={Uri.EscapeDataString(query)}&mode={searchMode}&phase={(full?"full":"instant")}",cancel);if(sequence!=querySequence||query!=search.Text.Trim())return true;rows.Clear();foreach(var item in reply["results"]?.AsArray()??[])if(item!=null)rows.Add(Row(item));total=rows.Count;offset=0;nextOffset=null;UpdatePager();SelectNavigation("search");UpdateChrome();message.Text=$"{rows.Count} matches · {reply["elapsed_ms"]?.GetValue<double>():0.0} ms  {reply["detail"]}";UpdateExpandedSearch();return reply["routing"]?["complete"]?.GetValue<bool>()==true;}
    Task EmptySearch(){if(!quick)return LoadFiles();rows.Clear();total=0;offset=0;nextOffset=null;UpdatePager();ShowList();message.Text="Search your files, or describe what you need.";return Task.CompletedTask;}
    FileRow Selected()=>(gridMode?icons.SelectedItem:files.SelectedItem) as FileRow??throw new InvalidOperationException("Select a file first.");
    async Task Preview(){var f=Selected();if(f.Kind=="folder"){await Browse(f.Device,f.Path);return;}var reply=await api.Post(f.Data["indexed_scope"]?.GetValue<bool>()==false?"explorer/preview":"workspace/preview",new JsonObject{["device"]=f.Device,["path"]=f.Path});await Dialog(f.Name,new ScrollViewer{MaxHeight=480,Content=new TextBlock{Text=reply["text"]?.ToString()??"No text preview available",TextWrapping=TextWrapping.Wrap,IsTextSelectionEnabled=true}});}
    async Task OpenFile(){var f=Selected();if(f.Kind=="folder"){await Browse(f.Device,f.Path);return;}if(api.Mode=="client"||f.Device!=status?["local_device"]?.ToString())throw new InvalidOperationException("Open this file on its source device. Remote download is pending candidate parity.");await Windows.System.Launcher.LaunchFileAsync(await Windows.Storage.StorageFile.GetFileFromPathAsync(f.Path));}

    async Task Devices(){await RefreshConfig();var panel=new StackPanel{Spacing=12,Margin=new Thickness(24)};panel.Children.Add(new TextBlock{Text="Devices and included folders",FontSize=24});foreach(var device in config!["devices"]!.AsArray()){var d=device!;panel.Children.Add(new TextBlock{Text=d["label"]!.ToString(),FontSize=17});foreach(var path in d["roots"]?.AsArray()??[]){var captured=path!.ToString();panel.Children.Add(Button(captured,()=>Browse(d["id"]!.ToString(),captured)));}}panel.Children.Add(Button("Include a folder on this computer",IncludeFolder));panel.Children.Add(Button("Connection settings",Connection));SetPage(panel);}
    async Task Crawling(){
        var data=await api.Get("crawl");var panel=new StackPanel{Spacing=16,Margin=new Thickness(24),MaxWidth=900,HorizontalAlignment=HorizontalAlignment.Left};
        panel.Children.Add(new TextBlock{Text="Crawling & learning",FontSize=28});panel.Children.Add(new TextBlock{Text="Build a local catalogue once, then check for changes on your schedule. Unchanged files reuse extracted text, descriptions, and embeddings.",TextWrapping=TextWrapping.Wrap});panel.Children.Add(new TextBlock{Text=data["disclosure"]?.ToString(),TextWrapping=TextWrapping.Wrap,Opacity=.7});
        var actions=new StackPanel{Orientation=Orientation.Horizontal,Spacing=8};actions.Children.Add(Button("Refresh status",Crawling));var cancel=Button("Cancel current job",async()=>{await api.Post("jobs/cancel");message.Text="Cancellation requested.";});actions.Children.Add(cancel);panel.Children.Add(actions);
        var summaries=new Dictionary<string,(TextBlock Text,Button Run)>();
        foreach(var value in data["folders"]?.AsArray()??[]){if(value is not JsonObject folder)continue;var f=folder.DeepClone().AsObject();var card=new StackPanel{Spacing=10,Padding=new Thickness(16)};card.Children.Add(new TextBlock{Text=f["root"]?.ToString(),TextWrapping=TextWrapping.Wrap,FontWeight=Microsoft.UI.Text.FontWeights.SemiBold});card.Children.Add(new TextBlock{Text=f["label"]?.ToString(),Opacity=.65});var summary=new TextBlock{TextWrapping=TextWrapping.Wrap,FontSize=12,Opacity=.8};card.Children.Add(summary);var buttons=new StackPanel{Orientation=Orientation.Horizontal,Spacing=8};buttons.Children.Add(Button("Folder settings",()=>EditCrawl(f)));var run=Button("Crawl now",async()=>{await api.Post("crawl/run",new JsonObject{["device"]=f["device"]?.DeepClone(),["root"]=f["root"]?.DeepClone()});await Crawling();});buttons.Children.Add(run);card.Children.Add(buttons);panel.Children.Add(new Border{Child=card,BorderThickness=new Thickness(1),CornerRadius=new CornerRadius(8),BorderBrush=new SolidColorBrush(Microsoft.UI.Colors.Gray)});summaries[f["device"]+"\n"+f["root"]]=(summary,run);}
        void Update(JsonNode result){static string When(JsonNode? n)=>double.TryParse(n?.ToString(),out var seconds)&&seconds>0?DateTimeOffset.FromUnixTimeSeconds((long)seconds).ToLocalTime().ToString("g"):"Not yet";bool running=result["running"]?.GetValue<bool>()??false;cancel.IsEnabled=running;foreach(var f in result["folders"]?.AsArray()??[]){if(f==null||!summaries.TryGetValue(f["device"]+"\n"+f["root"],out var row))continue;var report=f["report"];var next=f["interval_minutes"]?.GetValue<int>()==0?"Manual":(f["next_run"]?.GetValue<double>()??0)>0?When(f["next_run"]):"Due";row.Text.Text=$"{f["state"]} · Last completed: {When(f["last_completed"])} · Next: {next}\n{report?["changed"]??JsonValue.Create(0)} changed · {report?["unchanged"]??JsonValue.Create(0)} reused";if(report?["errors"] is JsonArray errors&&errors.Count>0)row.Text.Text+=$"\n{errors.Count} issue(s): {errors[0]?["reason"]}";row.Run.IsEnabled=!running;}message.Text=running?$"Working · {result["progress"]?["phase"]}. Search stays available.":"Schedules run while the coordinator is open. Overdue checks are combined.";}
        if(summaries.Count==0)panel.Children.Add(new TextBlock{Text="Include a folder in Devices to choose its crawl schedule."});SetPage(panel);Update(data);bool loading=false;var timer=new DispatcherTimer{Interval=TimeSpan.FromSeconds(5)};crawlTimer=timer;timer.Tick+=async(_,_)=>{if(loading)return;loading=true;try{var result=await api.Get("crawl");if(crawlTimer==timer)Update(result);}catch(Exception ex){if(crawlTimer==timer)message.Text=ex.Message;}finally{loading=false;}};timer.Start();
    }
    async Task EditCrawl(JsonObject f){
        var presets=new[]{0,15,60,360,1440,10080,-1};var minutes=f["interval_minutes"]?.GetValue<int>()??360;var selected=Array.IndexOf(presets,minutes);var schedule=new ComboBox{Header="Check for changes",ItemsSource=new[]{"Manual","Every 15 minutes","Hourly","Every 6 hours","Daily","Weekly","Custom…"},SelectedIndex=selected<0?6:selected,HorizontalAlignment=HorizontalAlignment.Stretch};var custom=new NumberBox{Header="Minutes (15–43200)",Minimum=15,Maximum=43200,SmallChange=15,Value=minutes>0?minutes:60,Visibility=schedule.SelectedIndex==6?Visibility.Visible:Visibility.Collapsed};schedule.SelectionChanged+=(_,_)=>custom.Visibility=schedule.SelectedIndex==6?Visibility.Visible:Visibility.Collapsed;
        var modes=new[]{"off"};var analysis=new ComboBox{Header="Learn from files",ItemsSource=new[]{"Native search · EmbeddingGemma + Laya"},SelectedIndex=Math.Max(0,Array.IndexOf(modes,f["analysis"]?.ToString())),HorizontalAlignment=HorizontalAlignment.Stretch};var panel=new StackPanel{Spacing=14};panel.Children.Add(schedule);panel.Children.Add(custom);panel.Children.Add(analysis);panel.Children.Add(new TextBlock{Text="Selected files are screened and extracted into the local catalogue. EmbeddingGemma indexes the cached passages; Laya checks search relevance. Original files remain unchanged.",TextWrapping=TextWrapping.Wrap});
        if(await Dialog("Folder crawl settings",panel,"Save")==ContentDialogResult.Primary){var value=schedule.SelectedIndex==6?custom.Value:presets[schedule.SelectedIndex];if(!double.IsFinite(value)||value!=Math.Truncate(value)||(value!=0&&(value<15||value>43200)))throw new InvalidOperationException("Choose manual or 15–43200 whole minutes.");await api.Post("crawl/settings",new JsonObject{["device"]=f["device"]?.DeepClone(),["root"]=f["root"]?.DeepClone(),["revision"]=f["revision"]?.DeepClone(),["interval_minutes"]=(int)value,["analysis"]=modes[analysis.SelectedIndex]});await Crawling();}
    }
    async Task Activity(){var data=await api.Get("workspace");var panel=new StackPanel{Margin=new Thickness(24),Spacing=12};panel.Children.Add(new TextBlock{Text="Activity",FontSize=28});foreach(var item in data["history"]?.AsArray()??[]){if(item==null)continue;var entry=item;var row=new StackPanel{Spacing=8};row.Children.Add(new TextBlock{Text=$"{entry["operation"]} · {entry["state"]}"});if(entry["state"]?.ToString()=="applied"&&(entry["operation"]?.ToString() is "rename" or "move" or "trash"))row.Children.Add(Button("Review undo",async()=>{var plan=await api.Post("workspace/undo",new JsonObject{["id"]=entry["id"]?.DeepClone()});var text=string.Join("\n",plan["items"]?.AsArray().Select(i=>$"{i?["source"]}\n→ {i?["destination"]}")??[]);if(await Dialog("Review undo",new ScrollViewer{MaxHeight=420,Content=new TextBlock{Text=text,TextWrapping=TextWrapping.Wrap}},"Apply undo")==ContentDialogResult.Primary){await api.Post("workspace/apply",new JsonObject{["id"]=plan["id"]?.DeepClone()});await Activity();}}));panel.Children.Add(row);}SetPage(panel);}
    async Task IncludeFolder(){if(api.Mode=="client")throw new InvalidOperationException("Include folders from their source computer. This client is connected to another coordinator.");var folder=await PickPath("Include a folder");if(folder==null)return;await ReviewCrawlScope(folder);}
    async Task Comments(){var f=Selected();var reply=await api.Post(f.Data["indexed_scope"]?.GetValue<bool>()==false?"explorer/preview":"workspace/preview",new JsonObject{["device"]=f.Device,["path"]=f.Path});var panel=new StackPanel{Spacing=12};foreach(var c in reply["metadata"]?["comments"]?.AsArray()??[])panel.Children.Add(new TextBlock{Text=$"{c?["author"]}: {c?["body"]}",TextWrapping=TextWrapping.Wrap});var author=new TextBox{Header="Your name",Text=Environment.UserName};var comment=new TextBox{Header="Comment",AcceptsReturn=true,MinHeight=90};panel.Children.Add(author);panel.Children.Add(comment);if(await Dialog("Comments · "+f.Name,panel,"Add comment")==ContentDialogResult.Primary)await api.Post("workspace/comment",new JsonObject{["device"]=f.Device,["path"]=f.Path,["author"]=author.Text,["body"]=comment.Text});}
    async Task Rename(){var f=Selected();var name=new TextBox{Header="New name",Text=f.Name};if(await Dialog("Rename "+f.Name,name,"Review")!=ContentDialogResult.Primary)return;var plan=await api.Post("workspace/plan",new JsonObject{["device"]=f.Device,["scope"]="filesystem",["operation"]="rename",["items"]=new JsonArray(new JsonObject{["path"]=f.Path,["name"]=name.Text})});if(await Dialog("Review rename",new TextBlock{Text=$"{f.Name}\n→ {name.Text}\n\nThe destination must be available. Changes are recorded for checked undo.",TextWrapping=TextWrapping.Wrap},"Apply rename")==ContentDialogResult.Primary){var receipt=await api.Post("workspace/apply",new JsonObject{["id"]=plan["id"]?.DeepClone()});message.Text=receipt["state"]?.ToString()??"Inspect the operation receipt.";await LoadFiles();}}
    async Task Connection(){var mode=new ComboBox{Header="Application mode",ItemsSource=new[]{"Use this computer as coordinator","Connect to another coordinator"},SelectedIndex=api.Mode=="client"?1:0};var endpoint=new TextBox{Header="Coordinator URL",Text=api.Endpoint};var token=new PasswordBox{Header="Pairing token",Password=api.Mode=="client"?api.Token:""};var panel=new StackPanel{Spacing=12};panel.Children.Add(mode);panel.Children.Add(endpoint);panel.Children.Add(token);if(await Dialog("Retriever connection",panel,"Connect")==ContentDialogResult.Primary){if(mode.SelectedIndex==0)await api.UseCoordinatorAsync();else await api.SaveClientAsync(endpoint.Text,token.Password);await RefreshConfig();await Navigate("files");}}
    void ShowCards(string title,JsonArray array,Func<JsonNode?,string> label){var panel=new StackPanel{Margin=new Thickness(24),Spacing=12};panel.Children.Add(new TextBlock{Text=title,FontSize=24});foreach(var v in array)panel.Children.Add(new TextBlock{Text=label(v),TextWrapping=TextWrapping.Wrap});if(array.Count==0)panel.Children.Add(new TextBlock{Text="Nothing here yet.",Opacity=.6});SetPage(panel);}
    void SetPage(UIElement content){StopDiskPolling();StopHealthPolling();crawlTimer?.Stop();crawlTimer=null;typing?.Cancel();Interlocked.Increment(ref querySequence);page.Children.Clear();var scroll=new ScrollViewer{Content=content};Grid.SetRowSpan(scroll,3);page.Children.Add(scroll);}
    async Task Settings()=>await SettingsDraft(null);
    async Task SettingsDraft(JsonNode? draft){if(draft is null)await RefreshConfig();var panel=new StackPanel{Spacing=16,Margin=new Thickness(24),MaxWidth=720,HorizontalAlignment=HorizontalAlignment.Left};panel.Children.Add(new TextBlock{Text="Settings",FontSize=28});panel.Children.Add(new TextBlock{Text="EmbeddingGemma and Laya run inside Retriever. Models, cached passages, vectors and metadata stay in the app data folder. Searches take priority over the next indexing batch.",TextWrapping=TextWrapping.Wrap});var pending=draft??config!.DeepClone();
        var status=await api.Get("status");
        foreach(var role in new[]{"embedding","relevance"}){var model=status[role];var card=new StackPanel{Spacing=8};card.Children.Add(new TextBlock{Text=role=="embedding"?"EmbeddingGemma · meaning search":"Laya · relevance ordering",FontSize=18});card.Children.Add(new TextBlock{Text=model?["detail"]?.ToString()??"Local retrieval model",TextWrapping=TextWrapping.Wrap});card.Children.Add(Button("Check local "+role,async()=>{var r=await api.Post("model-probe",new JsonObject{["purpose"]=role});message.Text=r["ok"]?.ToString()=="true"?"Local model verified":"Model check failed";}));panel.Children.Add(card);}
        panel.Children.Add(new TextBlock{Text="App data: "+status["storage"]?["root"]?.ToString(),TextWrapping=TextWrapping.Wrap,IsTextSelectionEnabled=true});
        panel.Children.Add(Button("Save settings",async()=>{var r=await api.Post("config",pending,revision);revision=r["revision"]!.ToString();message.Text="Settings saved. Start indexing when ready.";}));panel.Children.Add(Button("Connection settings",Connection));panel.Children.Add(Button("Updates",Updates));SetPage(panel);
    }
    async Task<ContentDialogResult> Dialog(string title,UIElement content,string? primary=null){var dialog=new ContentDialog{XamlRoot=root.XamlRoot,RequestedTheme=root.RequestedTheme,Title=title,Content=content,CloseButtonText=primary==null?"Done":"Cancel",PrimaryButtonText=primary??"",DefaultButton=primary==null?ContentDialogButton.Close:ContentDialogButton.Primary};dialogVisible=true;try{return await dialog.ShowAsync();}finally{dialogVisible=false;}}
}

