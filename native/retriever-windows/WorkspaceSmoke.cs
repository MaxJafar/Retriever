using Microsoft.UI.Xaml.Controls;
using System.Text.Json.Nodes;

namespace Retriever.Native;

public sealed partial class MainWindow {
#if RETRIEVER_TESTING
    bool fixtureInteractive;
    async Task WorkspaceTestsIfRequested(){
        if(Environment.GetEnvironmentVariable("RETRIEVER_PROFILE")?.StartsWith("workspace-smoke-health-offline")!=true)return;
        await File.WriteAllTextAsync(Path.Combine(api.StatePath,"desktop-status.json"),new JsonObject{["tray"]=desktop?.TrayAvailable,["hotkey"]=desktop?.HotkeyRegistered,["fallback"]=desktop?.UsesFallback,["tabs"]=tabStrip.TabItems.Count}.ToJsonString());
        if(Environment.GetEnvironmentVariable("RETRIEVER_FIXTURE_RESUME")=="1"){
            if(Environment.GetEnvironmentVariable("RETRIEVER_FIXTURE_CHECK_EXIT")=="1"){
                AppWindow.Hide();var hidden=!AppWindow.IsVisible;RestoreMain();await Task.Delay(100);
                var restored=AppWindow.IsVisible;PersistTabs();
                await File.WriteAllTextAsync(Path.Combine(api.StatePath,"lifecycle-status.json"),new JsonObject{["hidden"]=hidden,["restored"]=restored,["tray"]=desktop?.TrayAvailable,["hotkey"]=desktop?.HotkeyRegistered,["fallback"]=desktop?.UsesFallback,["quit_requested"]=true,["physical_input"]=false}.ToJsonString());Quit();return;
            }
            fixtureInteractive=true;Title="Retriever — Fixture preview";
            message.Text="Resumed fixture preview · live health blocked";return;
        }
        int checks=0;void Require(bool value,string description){if(!value)throw new InvalidOperationException(description);checks++;}
        var result=new JsonObject{["passed"]=false,["os_keyboard_simulated"]=false,["production_state_modified"]=false};
        try {
            var fixtures=Environment.GetEnvironmentVariable("RETRIEVER_FIXTURES")??throw new InvalidOperationException("Missing synthetic fixtures");
            var local=config!["devices"]!.AsArray().First(d=>d?["transport"]?.ToString()=="local")!;var device=local["id"]!.ToString();local["roots"]=new JsonArray(fixtures);config["configured"]=true;
            foreach(var role in new[]{"embedding","relevance","vision","llm"})config["search"]![role]!["provider"]="disabled";
            await api.Post("config",config.DeepClone(),revision);await api.Post("sync");
            for(int i=0;i<150;i++){await Task.Delay(100);if((await api.Get("status"))["syncing"]?.GetValue<bool>()!=true)break;}
            await RefreshConfig();await Navigate("files");Require(rows.Count>=9,"Fixture catalogue empty");
            Require(navButtons.ContainsKey("health")&&navButtons.ContainsKey("disk")&&!navButtons.ContainsKey("search"),"Sidebar insights or single search contract failed");
            search.Text="telescope Jupiter";await Search(true);Require(rows[0].Name=="astronomy.pdf","Synthetic search failed");
            var chromeUnloads=0;Microsoft.UI.Xaml.RoutedEventHandler unloaded=(_,_)=>chromeUnloads++;explorerChrome.Unloaded+=unloaded;
            ShowList();ShowList();await Task.Delay(75);explorerChrome.Unloaded-=unloaded;
            Require(chromeUnloads==0,"Refreshing results detached the tab strip");
            Require(TabCycleStep(Windows.System.VirtualKey.Tab,true,false,false)==1,"Ctrl+Tab route missing");
            Require(TabCycleStep(Windows.System.VirtualKey.Tab,true,true,false)==-1,"Reverse tab route missing");
            Require(TabCycleStep(Windows.System.VirtualKey.Tab,false,false,false)==0&&TabCycleStep(Windows.System.VirtualKey.Tab,true,false,true)==0,"Normal/Alt tab navigation intercepted");
            dialogVisible=true;Require(TabCycleStep(Windows.System.VirtualKey.Tab,true,false,false)==0,"Tab cycle escaped a modal");dialogVisible=false;
            Require(PropertyChoiceIndex(["No value","Review"],"No value")==1&&PropertyChoiceIndex(["No value","Review"],null)==0,"Select placeholder hides a valid option");
            var original=activeTab!;await NewTab();Require(tabStrip.TabItems.Count==2&&search.Text.Length==0,"New tab inherited search");
            var folder=activeTab!;await Browse(device,fixtures);gridMode=true;visibleColumns.Add("priority");sort="size";await LoadFiles();
            await CycleTab(1);Require(activeTab==original&&search.Text=="telescope Jupiter","Forward tab cycle lost query");await CycleTab(-1);Require(activeTab==folder&&gridMode,"Reverse tab cycle lost view");
            await ActivateTab(original);Require(search.Text=="telescope Jupiter"&&rows[0].Name=="astronomy.pdf","Search tab restoration failed");Require(!gridMode&&sort=="name","View state leaked across tabs");
            await ActivateTab(folder);Require(gridMode&&visibleColumns.Contains("priority")&&backStack.Count>0,"Folder tab state lost");
            await CloseTab(folder);Require(tabStrip.TabItems.Count==1,"Close tab failed");await ReopenTab();Require(tabStrip.TabItems.Count==2&&gridMode,"Reopen tab state lost");
            var restored=activeTab!;((ExplorerTab)restored.Tag).Pinned=true;await CloseTab(restored);Require(tabStrip.TabItems.Count==2,"Pinned tab closed");((ExplorerTab)restored.Tag).Pinned=false;
            await MoveTab(restored,-1);Require(ReferenceEquals(tabStrip.TabItems[0],restored),"Tab reorder failed");PersistTabs();
            await RestoreTabs();Require(tabStrip.TabItems.Count==2&&gridMode&&sort=="size","Session restart restoration failed");
            var property=await api.Post("workspace/property",new JsonObject{["name"]="Project",["kind"]="select",["options"]=new JsonArray("Sequoia","Moonstone")});
            var path=Path.Combine(fixtures,"budget.xlsx");var meta=await api.Post("workspace/inspect",new JsonObject{["device"]=device,["path"]=path});
            var body=new JsonObject{["device"]=device,["path"]=path,["revision"]=meta["revision"]!.DeepClone(),["tags"]=new JsonArray("Launch","Review"),["properties"]=new JsonObject{["priority"]="high",[property["id"]!.ToString()]="Sequoia"}};
            var saved=await api.Post("workspace/metadata",body);Require(saved["priority"]?.ToString()=="high","Priority missing");
            var comment=await api.Post("workspace/comment",new JsonObject{["device"]=device,["path"]=path,["author"]="Reviewer",["body"]="Workspace acceptance review"});Require(comment["comments"]!.AsArray().Count==1,"Comment missing");
            bool rejected=false;try{await api.Post("workspace/metadata",body);}catch(InvalidOperationException e){rejected=e.Message.StartsWith("conflict:");}Require(rejected,"Stale property save accepted");
            var filters=new JsonObject{["match"]="all",["rules"]=new JsonArray(new JsonObject{["field"]="tags",["operator"]="contains",["value"]="Launch"},new JsonObject{["field"]="priority",["operator"]="is",["value"]="high"})};
            var collection=await api.Post("workspace/library",new JsonObject{["filters"]=filters.DeepClone()});Require(collection["total"]!.GetValue<int>()==1,"Collection filters failed");
            await api.Post("workspace/view",new JsonObject{["name"]="Launch priorities",["kind"]="view",["body"]=new JsonObject{["filters"]=filters.DeepClone()}});await RefreshSidebar();
            Require(navButtons.Keys.Any(k=>k.StartsWith("view:")),"Saved collection missing from sidebar");
            await Browse(device,fixtures);
            var folderView=await api.Post("workspace/view",new JsonObject{["name"]="Fixture folder",["kind"]="view",["body"]=SavedViewBody()});
            await Navigate("files");await OpenView(folderView);
            Require(route=="browse"&&location["path"]?.ToString()==fixtures,"Saved folder expanded to All files");
            await Navigate("files");gridMode=false;BuildFileColumns();ShowList();
            await TestPropertyPopovers(Require);
            await TestExplorerMilestone(Require);
            try{await RenderWorkspaceTest(root,"workspace-tabs.png");result["app_view_rendered"]=true;files.SelectedItem=rows.First(r=>r.Name=="budget.xlsx");await Properties();result["property_view_rendered"]=File.Exists(Path.Combine(api.StatePath,"workspace-properties.png"));}catch(Exception e){result["app_view_render_error"]=e.Message;}
            var session=JsonNode.Parse(await File.ReadAllTextAsync(TabsPath))!;
            foreach(var tab in session["tabs"]!.AsArray()){tab!["state"]!["offset"]="invalid";tab["state"]!["grid"]="invalid";}
            await File.WriteAllTextAsync(TabsPath,session.ToJsonString());await RestoreTabs();
            Require(offset==0&&!gridMode&&rows.Count>=9,"Invalid saved view settings were not recovered");
            await Browse(device,fixtures);await OpenDiskSpace();
            Require(route=="disk"&&page.Children.Contains(diskPanel!),"Disk analysis tab did not open");
            location["root"]=fixtures;await StartDiskScan();
            for(int i=0;i<150&&diskResult?["scan"]?["state"]?.ToString()=="scanning";i++){await Task.Delay(100);await RefreshDisk();}
            Require(diskRows.Count>=9&&diskResult?["scan"]?["state"]?.ToString()=="complete","Disk scan did not populate native rows");
            Require(diskMap.Children.Count>0,"Disk treemap is empty");
            var realCapacity=diskResult!.DeepClone();
            api.CapacityTestFixture=JsonNode.Parse(await File.ReadAllTextAsync(Path.Combine(Path.GetDirectoryName(Environment.GetEnvironmentVariable("RETRIEVER_HEALTH_FIXTURES"))!,"disk-capacity.json")))!;
            Require((await api.Get("disk"))["scans"]?.AsArray().Count==1,"Capacity preview sidebar discovery failed");
            location["node"]=2;await RefreshDisk();Require(diskPath.Text.Contains("Projects")&&diskRows.Any(r=>r.Name=="budget.xlsx")&&diskUp!.IsEnabled,"Capacity fixture folder navigation failed");
            Require(!diskScan!.IsEnabled,"Capacity fixture enabled live scanning");bool scanBlocked=false;try{await StartDiskScan();}catch(InvalidOperationException){scanBlocked=true;}Require(scanBlocked,"Capacity fixture scan bypassed interception");
            location["node"]=1;location["mode"]="largest";await RefreshDisk();Require(diskRows.Count>0&&diskRows.All(r=>!r.Folder),"Capacity largest-files preview failed");var largestCount=diskRows.Count;
            location["mode"]="older";await RefreshDisk();Require(diskRows.Count>0&&diskRows.Count<largestCount,"Capacity age preview did not filter");
            location["mode"]="children";
            await RefreshDisk();Require(capacityOverview.Children.Count==3,"Capacity dashboard cards missing");await RenderWorkspaceTest(root,"disk-capacity.png");await RenderNarrowWorkspaceTest("disk-capacity-narrow.png");await ToggleTheme();await RenderWorkspaceTest(root,"disk-capacity-light.png");await ToggleTheme();api.CapacityTestFixture=null;diskResult=realCapacity;await RefreshDisk();
            var diskTab=activeTab!;var scanId=location["scan"]?.ToString();
            diskList.SelectedItem=diskRows.First(r=>r.Name=="budget.xlsx");await FindDiskRelated();
            Require(route=="library"&&search.Text=="budget"&&rows.Any(r=>r.Name=="budget.xlsx"),"Disk-to-search handoff failed");
            await ActivateTab(diskTab);Require(route=="disk"&&location["scan"]?.ToString()==scanId,"Disk tab lost its snapshot");
            location["mode"]="largest";await RefreshDisk();Require(diskRows.All(r=>!r.Folder),"Largest-files mode includes folders");
            await RenderWorkspaceTest(root,"disk-space.png");
            PersistTabs();await RestoreTabs();Require(route=="disk"&&location["scan"]?.ToString()==scanId,"Disk session restoration failed");
            search.Text="Sequoia budget";await Search(true);Require(route=="library"&&!page.Children.Contains(diskPanel!)&&rows[0].Name=="budget.xlsx","Searching left disk chrome attached");
            Require(DiskBytes(1024)=="1.0 KiB"&&DiskBytes(-1024).Contains("1.0 KiB"),"Binary size units are inconsistent");
            bool missingFixtureBlocked=false;
            try{await api.Post("health/refresh",new JsonObject());}catch(InvalidOperationException){missingFixtureBlocked=true;}
            Require(missingFixtureBlocked,"Test build allowed a health request without fixtures");
            api.HealthTestProvider=new HealthFixtures(Environment.GetEnvironmentVariable("RETRIEVER_HEALTH_FIXTURES")??throw new InvalidOperationException("Missing health fixture path"));
            bool fixtureMonitoringBlocked=false;
            try{await api.Post("health/settings",new JsonObject{{"interval_minutes",1}});}catch(InvalidOperationException){fixtureMonitoringBlocked=true;}
            Require(fixtureMonitoringBlocked,"Fixture provider allowed live monitoring");
            await OpenDriveHealth();Require(route=="health"&&page.Children.Contains(healthPanel!),"Drive health did not open in a tab");
            Require(!healthRefresh!.IsEnabled&&!healthInterval!.IsEnabled,"Blocked collection controls were enabled");
            Require(healthNotice.Text.Contains("SYNTHETIC"),"Fixture banner missing");
            Require((healthData?["drives"]?.AsArray().Count??0)==4,"Fixture drives not listed");
            Require(healthDetail?["drive"]?["status"]!=null,"Drive health missing explicit status");
            Require(healthData?["model_calls"]?.ToString()=="0","Health triggered inference");
            Require(healthOverview.Children.Count==3,"Health dashboard cards missing");
            var healthTab=activeTab!;var driveId=location["drive"]?.ToString();
            var driveChoices=healthDrives!.Items.OfType<ComboBoxItem>().ToArray();
            healthDrives.IsDropDownOpen=true;await RefreshHealth();Require(driveChoices.SequenceEqual(healthDrives.Items.OfType<ComboBoxItem>()),"Polling replaced an open drive dropdown");
            healthDrives.SelectedIndex=1;await Task.Delay(150);healthDrives.IsDropDownOpen=false;await Task.Delay(150);Require(healthSummary.Text.Contains("Caution"),"Native drive selection did not switch to HDD");
            Require(driveChoices.SequenceEqual(healthDrives.Items.OfType<ComboBoxItem>()),"Drive selection rebuilt native containers");
            location["drive"]=driveId;await RefreshHealth();
            await RenderWorkspaceTest(root,"drive-health.png");
            await RenderNarrowWorkspaceTest("drive-health-narrow.png");
            location["drive"]=new string('2',64);await RefreshHealth();Require(healthSummary.Text.Contains("Caution")&&healthRows.Count==5,"HDD caution fixture did not render");
            await RenderWorkspaceTest(root,"drive-health-hdd.png");
            location["drive"]=new string('3',64);await RefreshHealth();Require(healthSummary.Text.Contains("Unknown")&&healthRows.Count==0,"Unsupported drive shown as healthy");
            location["drive"]=new string('4',64);await RefreshHealth();Require(healthSummary.Text.Contains("Disconnected"),"Disconnected drive missing stale label");
            location["drive"]=driveId;await RefreshHealth();healthMode!.SelectedIndex=2;Require(healthRows.Count==2,"Alert and recovery fixture missing");
            healthMode!.SelectedIndex=1;Require(healthRows.Count>0,"Health history missing");
            PersistTabs();await RestoreTabs();Require(route=="health"&&location["drive"]?.ToString()==driveId&&healthMode.SelectedIndex==1,"Health tab did not restore selection and mode");
            healthMode.SelectedIndex=0;search.Text="Sequoia budget";await Search(true);
            Require(route=="library"&&!page.Children.Contains(healthPanel!)&&rows[0].Name=="budget.xlsx","Health search transition failed");
            Require(api.HealthTestProvider.Requests>0,"Health fixture provider was bypassed");result["health_fixture_only"]=true;result["live_health_collection"]=false;
            await TestDiskBurner(Require);result["disk_burner_fixture_only"]=true;
            await TestExplorerFoundation(Require);result["folder_associations_tested_in_memory"]=true;
            await Browse(device,fixtures);var folderCount=tabStrip.TabItems.Count;var folderId=activeTab;
            await NewTab();await Browse(device,fixtures);Require(activeTab==folderId&&tabStrip.TabItems.Count==folderCount+1,"Folder click did not reuse its existing tab");
            await NewTab(FolderTabState(device,fixtures));Require(activeTab!=folderId&&tabStrip.TabItems.Count==folderCount+2,"Explicit new folder tab was reused");
            await OpenDiskBurner();var burnerId=activeTab;var countBefore=tabStrip.TabItems.Count;
            await OpenDiskAssistant();await OpenDiskBurner();await OpenDiskBurner();Require(tabStrip.TabItems.Count==countBefore&&route=="burner","Repeated utility click created extra tabs");
            await NewTab();await OpenDiskBurner();Require(activeTab==burnerId&&tabStrip.TabItems.Count==countBefore+1,"Existing utility tab was not reused");
            Require(DestinationKey(FolderTabState(device,"C:\\Users\\Example\\"))==DestinationKey(FolderTabState(device,"c:/users/example")),"Windows folder identities are case sensitive");
            utilitiesCollapsed=true;SaveFolderTree();await RefreshSidebar();Require(!navButtons.ContainsKey("health"),"Collapsed Utilities remain visible");utilitiesCollapsed=false;SaveFolderTree();await RefreshSidebar();Require(navButtons["burner"].ContextFlyout is MenuFlyout,"Utility new-tab menu missing");
            result["passed"]=true;
        }catch(Exception e){result["error"]=e.ToString();}
        result["native_checks"]=checks;await File.WriteAllTextAsync(Path.Combine(api.StatePath,"workspace-native-smoke.json"),result.ToJsonString(new(){WriteIndented=true}));
        if(result["passed"]?.GetValue<bool>()==true&&Environment.GetEnvironmentVariable("RETRIEVER_FIXTURE_STAY_OPEN")=="1"){
            // Explicit test-only preview: the same fail-closed providers stay in place.
            fixtureInteractive=true;
            api.CapacityTestFixture=JsonNode.Parse(await File.ReadAllTextAsync(Path.Combine(Path.GetDirectoryName(Environment.GetEnvironmentVariable("RETRIEVER_HEALTH_FIXTURES"))!,"disk-capacity.json")))!;
            visibleColumns.Clear();foreach(var key in new[]{"modified","type","tags","priority"})visibleColumns.Add(key);
            sort="name";gridMode=false;BuildFileColumns();await Navigate("files");
            Title="Retriever — Fixture preview";message.Text="Fixture preview · synthetic files and dashboards · live health blocked";
            return;
        }
        Quit();
    }
    void ObserveWorkspaceDialogForTest(ContentDialog dialog){
        if(Environment.GetEnvironmentVariable("RETRIEVER_PROFILE")?.StartsWith("workspace-smoke-health-offline")!=true)return;
        dialog.Opened+=async(_,_)=>{try{await RenderWorkspaceTest(dialog,"workspace-properties.png");}catch(Exception e){await File.WriteAllTextAsync(Path.Combine(api.StatePath,"property-render-error.txt"),e.ToString());}finally{dialog.Hide();}};
    }
    static IEnumerable<Button> TestButtons(Microsoft.UI.Xaml.DependencyObject parent){if(parent is Button b)yield return b;for(int i=0;i<Microsoft.UI.Xaml.Media.VisualTreeHelper.GetChildrenCount(parent);i++)foreach(var item in TestButtons(Microsoft.UI.Xaml.Media.VisualTreeHelper.GetChild(parent,i)))yield return item;}
    static string TestButtonText(Button b)=>b.Content is StackPanel p?string.Join("",p.Children.OfType<TextBlock>().Select(t=>t.Text)):b.Content?.ToString()??"";
    static void TestInvoke(Button b){var peer=new Microsoft.UI.Xaml.Automation.Peers.ButtonAutomationPeer(b);((Microsoft.UI.Xaml.Automation.Provider.IInvokeProvider)peer.GetPattern(Microsoft.UI.Xaml.Automation.Peers.PatternInterface.Invoke)).Invoke();}
    async Task<Button> TestFileCell(string name,string key){
        files.ScrollIntoView(rows.First(r=>r.Name==name));
        for(int i=0;i<60;i++){root.UpdateLayout();var cell=TestButtons(files).FirstOrDefault(b=>b.Tag?.ToString()=="cell:"+key&&b.DataContext is FileRow r&&r.Name==name);if(cell!=null)return cell;await Task.Delay(50);}
        throw new InvalidOperationException($"Property cell was not realized: {name} / {key}");
    }
    async Task TestPropertyPopovers(Action<bool,string> require){
        var file=rows.First(r=>r.Name=="budget.xlsx");visibleColumns.Add("tags");visibleColumns.Add("priority");await LoadPropertyDefinitions();BuildFileColumns();ShowList();await Task.Delay(450);
        var cell=await TestFileCell(file.Name,"tags");TestInvoke(cell);await Task.Delay(450);require(activePopover!=null,"Table cell did not open a property popover");
        var popup=activePopover!;var content=(StackPanel)popup.Content;var body=content.Children.OfType<StackPanel>().First();var picker=body.Children.OfType<StackPanel>().First();var filter=picker.Children.OfType<TextBox>().First();filter.Text="Design";await Task.Delay(50);var create=TestButtons(picker).First(b=>TestButtonText(b).StartsWith("Create “Design”"));TestInvoke(create);
        async Task<bool> HasTag(bool expected){for(int i=0;i<30;i++){await Task.Delay(50);var meta=await api.Post("workspace/inspect",new JsonObject{["device"]=file.Device,["path"]=file.Path});if((meta["tags"]?.AsArray().Any(v=>v?.ToString()=="Design")==true)==expected)return true;}return false;}
        require(await HasTag(true),"Created tag did not save from its popover");require(filter.FocusState!=Microsoft.UI.Xaml.FocusState.Unfocused&&!filter.IsReadOnly,"Creating a tag lost keyboard focus");await RenderWorkspaceTest(content,"tag-popover.png");var remove=TestButtons(picker).First(b=>TestButtonText(b)=="Design ×");TestInvoke(remove);require(await HasTag(false),"Removing a tag did not persist");await ClosePopover();
        await SortBy("name");var nameHeader=columnHeader.Children.OfType<Button>().First(b=>b.Tag?.ToString()=="name");require(((StackPanel)nameHeader.Content).Children.OfType<FontIcon>().Last().Glyph==(direction=="desc"?"\uE74B":"\uE74A"),"Column sort indicator is stale");
        var custom=await api.Post("workspace/property",new JsonObject{["name"]="Milestone",["kind"]="multi_select",["options"]=new JsonArray()});var extended=await AddPropertyOption(custom["id"]!.ToString(),"Review");require(extended["options"]!.AsArray().Count==1,"Inline property option was not created");await AddPropertyOption(custom["id"]!.ToString(),"review");require(propertyDefinitions.First(d=>d?["id"]?.ToString()==custom["id"]?.ToString())!["options"]!.AsArray().Count==1,"Inline creation duplicated an existing choice");
        visibleColumns.Add("prop:"+custom["id"]);BuildFileColumns();require(columnHeader.Children.OfType<Button>().Any(b=>TestButtonText(b)=="Milestone"),"Custom property column missing");await Task.Delay(100);require(tableScroll.ScrollableWidth>0,"Wide property table has no horizontal scrolling");tableScroll.ChangeView(tableScroll.ScrollableWidth,null,null,true);await Task.Delay(150);require(Math.Abs(tableScroll.HorizontalOffset-headerScroll.HorizontalOffset)<2,"Property header scroll is misaligned");tableScroll.ChangeView(0,null,null,true);
        await SmartFilters();await Task.Delay(150);require(activePopover!=null&&dialogVisible,"Filter did not open a non-modal flyout");await RenderWorkspaceTest((Microsoft.UI.Xaml.UIElement)activePopover!.Content,"filter-popover.png");await ClosePopover();require(!dialogVisible,"Dismissing a popover left keyboard navigation blocked");
        await Columns();await Task.Delay(250);require(activePopover!=null,"View settings did not open");await RenderWorkspaceTest((Microsoft.UI.Xaml.UIElement)activePopover!.Content,"view-settings-popover.png");
        TestInvoke(TestButtons((Microsoft.UI.Xaml.DependencyObject)activePopover.Content).First(b=>TestButtonText(b)=="New property"));
        for(int i=0;i<20;i++){await Task.Delay(50);if(activePopover?.Content is StackPanel p&&p.Children.OfType<StackPanel>().Any())break;}
        require(activePopover?.Content is StackPanel,"New property picker did not open from view settings");var createBody=((StackPanel)activePopover!.Content).Children.OfType<StackPanel>().First();createBody.Children.OfType<TextBox>().First().Text="Notes";
        await RenderWorkspaceTest((Microsoft.UI.Xaml.UIElement)activePopover.Content,"new-property-popover.png");TestInvoke(TestButtons(createBody).First(b=>TestButtonText(b)=="Text"));
        for(int i=0;i<30&&!propertyDefinitions.Any(d=>d?["name"]?.ToString()=="Notes");i++)await Task.Delay(50);
        require(propertyDefinitions.Any(d=>d?["name"]?.ToString()=="Notes"),"New property did not save through the picker");await Task.Delay(350);
        var customCell=await TestFileCell(file.Name,"prop:"+custom["id"]);TestInvoke(customCell);await Task.Delay(250);
        require(activePopover!=null,"Custom property cell did not open");var customPicker=TestButtons((Microsoft.UI.Xaml.DependencyObject)activePopover!.Content).First(b=>TestButtonText(b)=="Review");TestInvoke(customPicker);
        bool customSaved=false;for(int i=0;i<25;i++){await Task.Delay(60);var metadata=await api.Post("workspace/inspect",new JsonObject{["device"]=file.Device,["path"]=file.Path});if(metadata["properties"]?[custom["id"]!.ToString()] is JsonArray choices&&choices.Any(c=>c?.ToString()=="Review")){customSaved=true;break;}}
        require(customSaved,"Custom multi-select cell did not save");await ClosePopover();await Task.Delay(100);
    }
    async Task TestExplorerMilestone(Action<bool,string> require){
        require(navButtons.ContainsKey("assistant"),"Disk Assistant sidebar shortcut missing");
        var originalConnection=(api.Endpoint,api.Token,api.Mode);bool connectionRejected=false;
        try{await api.SaveClientAsync(api.Endpoint,"invalid-fixture-token");}catch(InvalidOperationException){connectionRejected=true;}
        require(connectionRejected&&(api.Endpoint,api.Token,api.Mode)==originalConnection,"Failed connection replaced the working coordinator");
        require((await api.Get("status"))["runtime"]?.ToString()=="rust","Failed pairing left the old connection unusable");
        require(FileRow.FormatDuration(3661)=="1:01:01"&&FileRow.FormatDuration(65)=="1:05","Video length formatting failed");
        files.SelectedItems.Clear();files.SelectedItems.Add(rows.First());require(rows.First().IsSelected&&batchActions!.IsEnabled,"Checkbox selection and bulk actions are out of sync");
        files.SelectedItems.Clear();require(!rows.Any(r=>r.IsSelected),"Clearing selection left stale checkbox state");
        search.Text="Sequoia";await Search(true);OpenSearchExpansion();await Task.Delay(200);
        require(searchExpanded&&expandedResults?.Items.Count>0,"Search expansion did not show results");require(searchExpansion!.Width>=500,"Expanded search area is too narrow");
        await RenderWorkspaceTest(root,"expanded-search.png");CloseSearchExpansion();require(!searchExpanded&&!dialogVisible,"Search expansion blocked keyboard navigation after dismissal");
        await DesktopPopover();await Task.Delay(300);require(activePopover!=null,"Desktop app popover missing");await ClosePopover();
        await ConnectionsPopover();await Task.Delay(300);require(activePopover!=null,"Connections popover missing");await ClosePopover();
        await ActivityPopover();await Task.Delay(300);require(activePopover!=null,"Activity popover missing");await ClosePopover();
        await AddRemoteDevice();await Task.Delay(300);require(activePopover!=null,"Remote setup popover missing");await RenderWorkspaceTest((Microsoft.UI.Xaml.UIElement)activePopover!.Content,"remote-setup.png");await ClosePopover();
        var fixtureFile=rows.First();var snapshot=await api.Post("devices/storage",new JsonObject{["device"]=fixtureFile.Device,["path"]=Environment.GetEnvironmentVariable("RETRIEVER_FIXTURES")});
        require(snapshot["files"]!.GetValue<int>()>=9&&snapshot["health_collection"]?.ToString()=="blocked"&&snapshot["model_calls"]!.GetValue<int>()==0,"Fixture storage snapshot did not remain metadata-only");
        var previous=activeTab!;await OpenDiskAssistant();require(route=="assistant"&&page.Children.Contains(explorerChrome),"Disk Assistant lost the approved tabs/header layout");
        await RenderWorkspaceTest(root,"disk-assistant.png");PersistTabs();await RestoreTabs();require(route=="assistant","Disk Assistant tab was not restored");
        await Back();require(search.Text=="Sequoia"&&rows.Any(r=>r.Name=="budget.xlsx"),"Back from Disk Assistant lost its search");await Navigate("files");
    }
    async Task RenderWorkspaceTest(Microsoft.UI.Xaml.UIElement element,string filename){
        // Render only this test application's XAML, never the screen or another app.
        await Task.Delay(350);root.UpdateLayout();
        var bitmap=new Microsoft.UI.Xaml.Media.Imaging.RenderTargetBitmap();await bitmap.RenderAsync(element);
        var pixels=await bitmap.GetPixelsAsync();var bytes=new byte[pixels.Length];
        using(var reader=Windows.Storage.Streams.DataReader.FromBuffer(pixels))reader.ReadBytes(bytes);
        var path=Path.Combine(api.StatePath,filename);await File.WriteAllBytesAsync(path,[]);
        var file=await Windows.Storage.StorageFile.GetFileFromPathAsync(path);
        using var stream=await file.OpenAsync(Windows.Storage.FileAccessMode.ReadWrite);
        var encoder=await Windows.Graphics.Imaging.BitmapEncoder.CreateAsync(Windows.Graphics.Imaging.BitmapEncoder.PngEncoderId,stream);
        encoder.SetPixelData(Windows.Graphics.Imaging.BitmapPixelFormat.Bgra8,Windows.Graphics.Imaging.BitmapAlphaMode.Premultiplied,(uint)bitmap.PixelWidth,(uint)bitmap.PixelHeight,96,96,bytes);await encoder.FlushAsync();
    }
    async Task RenderNarrowWorkspaceTest(string filename){var previous=AppWindow.Size;try{AppWindow.Resize(new Windows.Graphics.SizeInt32(820,760));await Task.Delay(200);await RenderWorkspaceTest(root,filename);}finally{AppWindow.Resize(previous);await Task.Delay(100);}}
    async Task TestDiskBurner(Action<bool,string> require){
        require(navButtons.ContainsKey("burner"),"Disk Burner sidebar shortcut missing");await OpenDiskBurner();
        require(route=="burner"&&page.Children.Contains(burnerPanel!),"Disk Burner did not open in the approved shell");
        require(message.Text.StartsWith("Disk Burner ·"),"Previous dashboard status leaked into Disk Burner");
        require(burnerData?["physical_writes"]?.GetValue<bool>()==false&&burnerStart?.IsEnabled==false,"Physical writing or empty rehearsal enabled");
        var sample=await api.Post("burner/sample");await InspectBurnerImage(sample["path"]!.ToString());
        for(int i=0;i<200&&BurnerRunning(burnerData?["current"]);i++){await Task.Delay(40);await RefreshBurner();}
        require(burnerData?["current"]?["state"]?.ToString()=="ready"&&burnerStart?.IsEnabled==true,"Sample image inspection failed");
        require(burnerSource.Text=="retriever-sample.img"&&burnerTargetDetail.Text.Contains("Fits"),"Image and capacity labels did not update");
        var selectedId=burnerData?["current"]?["id"]?.ToString();
        var pendingSelection=new TaskCompletionSource<string?>();var selecting=SelectBurnerImage(()=>pendingSelection.Task);
        await RefreshBurner();require(burnerChoosing&&burnerChoose?.IsEnabled==false&&burnerSample?.IsEnabled==false&&burnerStart?.IsEnabled==false,"Image selection did not lock competing source and write actions");
        bool duplicatePicker=false;await SelectBurnerImage(()=>{duplicatePicker=true;return Task.FromResult<string?>(null);});
        require(!duplicatePicker,"A second picker opened while a selection was pending");
        pendingSelection.SetResult(null);await selecting;await RefreshBurner();
        require(!burnerChoosing&&burnerChoose?.IsEnabled==true&&burnerStart?.IsEnabled==true&&burnerData?["current"]?["id"]?.ToString()==selectedId,"Cancel changed the previous inspection or left actions disabled");
        await SelectBurnerImage(()=>Task.FromException<string?>(new IOException("Synthetic picker failure")));await RefreshBurner();await RefreshBurner();
        require(burnerNotice.Text.Contains("Synthetic picker failure")&&message.Text.Contains("needs attention"),"Polling hid an image-picker error");
        require(burnerChoose?.IsEnabled==true&&burnerStart?.IsEnabled==false,"Picker error did not permit retry or allowed a stale image to be rehearsed");
        await SelectBurnerImage(()=>Task.FromResult<string?>(null));await RefreshBurner();
        require(burnerNotice.Text.Contains("Synthetic picker failure"),"Cancelling a retry discarded the unresolved source error");
        burnerChecksum.Text="invalid";await SelectBurnerImage(()=>Task.FromResult<string?>(sample["path"]!.ToString()));await RefreshBurner();
        require(burnerActionError!=null&&burnerData?["current"]?["id"]?.ToString()==selectedId&&burnerStart?.IsEnabled==false,"Immediate inspection rejection was not retained");
        burnerChecksum.Text="";
        // Exercise the actual Windows StorageFile path returned to the shared callback.
        // This does not stand in for the separate manual native-picker acceptance check.
        await SelectBurnerImage(async()=>{var file=await Windows.Storage.StorageFile.GetFileFromPathAsync(sample["path"]!.ToString());return file.Path;});
        for(int i=0;i<200&&BurnerRunning(burnerData?["current"]);i++){await Task.Delay(40);await RefreshBurner();}
        require(burnerActionError==null&&burnerData?["current"]?["state"]?.ToString()=="ready"&&burnerStart?.IsEnabled==true,"StorageFile selection did not recover after a picker/validation error");
        await ReviewBurner();await Task.Delay(100);require(activePopover!=null&&dialogVisible,"Rehearsal review is not an anchored popover");await ClosePopover();
        var id=burnerData!["inspection"]!["id"]!.ToString();await api.Post("burner/start",new JsonObject{["inspection_id"]=id,["target_id"]="sandbox-8"});await RefreshBurner();
        for(int i=0;i<200&&BurnerRunning(burnerData?["current"]);i++){await Task.Delay(40);await RefreshBurner();}
        var result=burnerData!["current"]!;
        require(result["state"]?.ToString()=="complete"&&Number(result["verified_bytes"])==4*1024*1024,"File rehearsal was not fully verified");
        require(result["verified_sha256"]?.ToString()==result["image"]?["sha256"]?.ToString(),"Read-back SHA-256 mismatch");
        await BurnerReceipt(result);await Task.Delay(80);require(activePopover!=null,"Receipt popover missing");await ClosePopover();
        await RenderWorkspaceTest(root,"disk-burner.png");await RenderNarrowWorkspaceTest("disk-burner-narrow.png");
        PersistTabs();await RestoreTabs();require(route=="burner"&&burnerData?["current"]?["state"]?.ToString()=="complete","Burner tab/receipt did not restore");
        search.Text="Sequoia";await Search(true);require(route=="library"&&!page.Children.Contains(burnerPanel!)&&burnerTimer==null,"Search did not leave Disk Burner cleanly");
    }
#else
    Task WorkspaceTestsIfRequested()=>Task.CompletedTask;
#endif
}
