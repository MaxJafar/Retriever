using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Markup;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using System.Text.Json.Nodes;
using Windows.UI;

namespace Retriever.Native;

// Keep this shell aligned with native/design/EXPLORER.md and its approved reference.
public sealed partial class MainWindow {
    readonly SolidColorBrush surface = Brush(0x1f1f1f), chrome = Brush(0x242424), line = Brush(0x383838);
    readonly SolidColorBrush accent = Brush(0xe5bd4c), selection = Brush(0x443d29);
    readonly SolidColorBrush hover=Brush(0x353535),pressed=Brush(0x454545),rowHover=Brush(0x2a2a2a),selectedHover=Brush(0x51472d);
    readonly StackPanel sidebarItems = new(), explorerChrome = new();
    readonly StackPanel explorerViews = new(){Orientation=Orientation.Horizontal,Spacing=2};
    readonly Grid explorerBody = new(), columnHeader = new();
    readonly TextBlock emptyLabel = new(){Text="No indexed files yet. Browse a device, or choose crawl folders in Disk Assistant.",TextWrapping=TextWrapping.Wrap,Opacity=.6,FontSize=13,HorizontalAlignment=HorizontalAlignment.Center,VerticalAlignment=VerticalAlignment.Center,MaxWidth=400,Margin=new Thickness(24)};
    readonly TextBlock sectionTitle = new(){Text="All files",FontSize=13,VerticalAlignment=VerticalAlignment.Center};
    readonly Dictionary<string,Button> navButtons = new();
    readonly HashSet<string> visibleColumns = ["modified","type","size","device"];
    readonly Stack<(string Route,JsonObject Location,string Title,string Nav,string Query)> forwardStack = new();
    Button? backButton,forwardButton,upButton,sortButton,filterButton,columnsButton;
    readonly ScrollViewer tableScroll=new(){HorizontalScrollBarVisibility=ScrollBarVisibility.Auto,HorizontalScrollMode=ScrollMode.Enabled,VerticalScrollBarVisibility=ScrollBarVisibility.Disabled,VerticalScrollMode=ScrollMode.Disabled,VerticalContentAlignment=VerticalAlignment.Stretch};
    readonly ScrollViewer headerScroll=new(){HorizontalScrollBarVisibility=ScrollBarVisibility.Hidden,HorizontalScrollMode=ScrollMode.Enabled,VerticalScrollBarVisibility=ScrollBarVisibility.Disabled,VerticalScrollMode=ScrollMode.Disabled};
    double minimumTableWidth=700;
    void SizeFileTable(){var width=Math.Max(minimumTableWidth,tableScroll.ActualWidth);files.Width=width;columnHeader.Width=width;}
    Border? sidebar;
    bool darkTheme=true,sidebarVisible=true;
    string selectedNavigation="files",currentTitle="All files";

    static SolidColorBrush Brush(uint rgb)=>new(Color.FromArgb(255,(byte)(rgb>>16),(byte)(rgb>>8),(byte)rgb));
    static TextBlock Label(string text,double size=12)=>new(){Text=text,FontSize=size,VerticalAlignment=VerticalAlignment.Center,TextTrimming=TextTrimming.CharacterEllipsis};
    static FontIcon Glyph(string glyph,double size=14)=>new(){Glyph=glyph,FontSize=size,FontFamily=new FontFamily("Segoe Fluent Icons")};
    Border Ruled(UIElement child, bool vertical=false)=>new(){Child=child,BorderBrush=line,BorderThickness=vertical?new Thickness(0,0,1,0):new Thickness(0,0,0,1)};
    Button FlatButton(string text,string glyph,Func<Task> action,bool iconOnly=false){
        var content=new StackPanel{Orientation=Orientation.Horizontal,Spacing=8,VerticalAlignment=VerticalAlignment.Center};
        if(glyph.Length>0)content.Children.Add(Glyph(glyph));
        if(!iconOnly)content.Children.Add(Label(text));
        var button=Button(text,action);button.Content=content;button.Padding=new Thickness(8,4,8,4);
        button.MinHeight=28;button.BorderThickness=new Thickness(0);button.Background=new SolidColorBrush(Microsoft.UI.Colors.Transparent);button.CornerRadius=new CornerRadius(3);
        AutomationProperties.SetName(button,text);ToolTipService.SetToolTip(button,text);return button;
    }
    void BuildShell(){
        root.RequestedTheme=ElementTheme.Dark;root.Background=quick?new SolidColorBrush(Color.FromArgb(180,31,31,31)):surface;
        root.Resources["ButtonBackgroundPointerOver"]=hover;
        root.Resources["ButtonBackgroundPressed"]=pressed;
        root.Resources["ButtonBackgroundDisabled"]=new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        foreach(var key in new[]{"AccentFillColorDefaultBrush","AccentFillColorSecondaryBrush","AccentFillColorTertiaryBrush","AccentTextFillColorPrimaryBrush"})root.Resources[key]=accent;
        root.Resources["ListViewItemBackgroundSelected"]=selection;
        root.Resources["ListViewItemBackgroundSelectedPointerOver"]=selectedHover;
        root.Resources["ListViewItemBackgroundPointerOver"]=rowHover;
        root.Resources["ListViewItemSelectionIndicatorBrush"]=accent;
        root.RowDefinitions.Add(new(){Height=GridLength.Auto});
        root.RowDefinitions.Add(new(){Height=new GridLength(1,GridUnitType.Star)});
        root.RowDefinitions.Add(new(){Height=GridLength.Auto});
        var header=new Grid{Height=68,Background=chrome,ColumnSpacing=12,Padding=new Thickness(8,8,142,8)};
        header.ColumnDefinitions.Add(new(){Width=new GridLength(quick?42:190)});
        header.ColumnDefinitions.Add(new(){Width=new GridLength(1,GridUnitType.Star)});
        header.ColumnDefinitions.Add(new(){Width=GridLength.Auto});
        var brand=new Grid();brand.ColumnDefinitions.Add(new(){Width=GridLength.Auto});brand.ColumnDefinitions.Add(new(){Width=new GridLength(1,GridUnitType.Star)});
        if(!quick)brand.Children.Add(FlatButton("Toggle sidebar","\uE700",()=>{sidebarVisible=!sidebarVisible;UpdateSidebarWidth();return Task.CompletedTask;},true));
        var identity=new StackPanel{Orientation=Orientation.Horizontal,Spacing=9,VerticalAlignment=VerticalAlignment.Center,Margin=new Thickness(4,0,0,0)};
        identity.Children.Add(new Image{Source=new BitmapImage(new Uri(Path.Combine(AppContext.BaseDirectory,"Retriever.png"))),Width=30,Height=30});
        if(!quick)identity.Children.Add(new TextBlock{Text="Retriever",FontSize=13,FontWeight=Microsoft.UI.Text.FontWeights.SemiBold,VerticalAlignment=VerticalAlignment.Center});
        Grid.SetColumn(identity,1);brand.Children.Add(identity);header.Children.Add(brand);
        ExtendsContentIntoTitleBar=true;SetTitleBar(identity);
        AppWindow.TitleBar.ButtonBackgroundColor=Microsoft.UI.Colors.Transparent;AppWindow.TitleBar.ButtonInactiveBackgroundColor=Microsoft.UI.Colors.Transparent;
        AppWindow.TitleBar.ButtonForegroundColor=Color.FromArgb(255,185,185,185);
        var searchFrame=new Grid{Background=surface,CornerRadius=new CornerRadius(10),BorderThickness=new Thickness(1),BorderBrush=line,Height=44};
        searchFrame.ColumnDefinitions.Add(new(){Width=new GridLength(40)});
        searchFrame.ColumnDefinitions.Add(new(){Width=new GridLength(1,GridUnitType.Star)});
        searchFrame.ColumnDefinitions.Add(new(){Width=GridLength.Auto});
        var searchIcon=Glyph("\uE721",18);searchIcon.Foreground=accent;searchFrame.Children.Add(searchIcon);
        search.PlaceholderText="Search your files, or describe what you need…";search.MinHeight=40;search.FontSize=14;search.BorderThickness=new Thickness(0);search.Background=new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        var searchStyle=new Style(typeof(TextBox));searchStyle.Setters.Add(new Setter(Control.VerticalContentAlignmentProperty,VerticalAlignment.Center));searchStyle.Setters.Add(new Setter(Control.PaddingProperty,new Thickness(0,8,0,8)));searchStyle.Setters.Add(new Setter(Control.BorderThicknessProperty,new Thickness(0)));search.TextBoxStyle=searchStyle;
        SetupSearchExpansion(searchFrame);
        search.Resources["TextControlBackground"]=new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        search.Resources["TextControlBackgroundFocused"]=new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        search.Resources["TextControlBorderBrushFocused"]=new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        AutomationProperties.SetName(search,"Search your files");Grid.SetColumn(search,1);searchFrame.Children.Add(search);
        var shortcut=new Border{Child=Label("Ctrl K",10),BorderBrush=line,BorderThickness=new Thickness(1),CornerRadius=new CornerRadius(3),Padding=new Thickness(5,2,5,2),Margin=new Thickness(8),VerticalAlignment=VerticalAlignment.Center};
        Grid.SetColumn(shortcut,2);searchFrame.Children.Add(shortcut);Grid.SetColumn(searchFrame,1);header.Children.Add(searchFrame);
        var headerActions=new StackPanel{Orientation=Orientation.Horizontal,Spacing=2,VerticalAlignment=VerticalAlignment.Center};
        if(!quick){headerActions.Children.Add(FlatButton("Switch theme","\uE706",ToggleTheme,true));headerActions.Children.Add(FlatButton("Crawling & learning","\uE9E9",EngineFoldersPopover,true));headerActions.Children.Add(FlatButton("Settings","\uE713",Settings,true));}
        Grid.SetColumn(headerActions,2);header.Children.Add(headerActions);root.Children.Add(Ruled(header));
        explorerBody.ColumnDefinitions.Add(new(){Width=new GridLength(quick?0:210)});explorerBody.ColumnDefinitions.Add(new(){Width=new GridLength(1,GridUnitType.Star)});
        sidebar=new Border{Background=chrome,BorderBrush=line,BorderThickness=new Thickness(0,0,1,0)};
        var sidebarGrid=new Grid();sidebarGrid.RowDefinitions.Add(new(){Height=new GridLength(1,GridUnitType.Star)});sidebarGrid.RowDefinitions.Add(new(){Height=GridLength.Auto});
        sidebarGrid.Children.Add(new ScrollViewer{Content=sidebarItems,HorizontalScrollBarVisibility=ScrollBarVisibility.Disabled,VerticalScrollBarVisibility=ScrollBarVisibility.Auto});
        var bottom=new Grid{Padding=new Thickness(4,10,4,8),RowSpacing=3};
        bottom.ColumnDefinitions.Add(new());bottom.ColumnDefinitions.Add(new());bottom.RowDefinitions.Add(new());bottom.RowDefinitions.Add(new());
        var footerActions=new(string,string,Func<Task>)[]{("Folder sync","\uE895",()=>Navigate("sync")),("Desktop app","\uE7F4",DesktopPopover),("Connections","\uE968",ConnectionsPopover),("Activity","\uE81C",ActivityPopover)};
        for(int i=0;i<footerActions.Length;i++){var a=footerActions[i];var b=FlatButton(a.Item1,a.Item2,a.Item3);((TextBlock)((StackPanel)b.Content).Children.Last()).FontSize=10;Grid.SetColumn(b,i%2);Grid.SetRow(b,i/2);bottom.Children.Add(b);}
        var bottomBorder=new Border{Child=bottom,BorderBrush=line,BorderThickness=new Thickness(0,1,0,0)};Grid.SetRow(bottomBorder,1);sidebarGrid.Children.Add(bottomBorder);sidebar.Child=sidebarGrid;explorerBody.Children.Add(sidebar);
        page.RowDefinitions.Add(new(){Height=GridLength.Auto});page.RowDefinitions.Add(new(){Height=new GridLength(1,GridUnitType.Star)});page.RowDefinitions.Add(new(){Height=GridLength.Auto});
        Grid.SetColumn(page,1);explorerBody.Children.Add(page);Grid.SetRow(explorerBody,1);root.Children.Add(explorerBody);
        message.FontSize=10;message.TextWrapping=TextWrapping.NoWrap;message.TextTrimming=TextTrimming.CharacterEllipsis;message.Margin=new Thickness(10,4,10,4);
        var statusRow=new Grid();statusRow.ColumnDefinitions.Add(new(){Width=new GridLength(1,GridUnitType.Star)});statusRow.ColumnDefinitions.Add(new(){Width=GridLength.Auto});statusRow.Children.Add(message);
        updateNotice=Button("Update available",Updates);updateNotice.FontSize=10;updateNotice.MinHeight=0;updateNotice.Height=22;updateNotice.Padding=new Thickness(10,0,10,0);updateNotice.Visibility=Visibility.Collapsed;Grid.SetColumn(updateNotice,1);statusRow.Children.Add(updateNotice);
        var statusBorder=new Border{Child=statusRow,Background=chrome,BorderBrush=line,BorderThickness=new Thickness(0,1,0,0)};Grid.SetRow(statusBorder,2);root.Children.Add(statusBorder);
        BuildExplorerChrome();BuildFileColumns();Content=root;UpdateSidebarWidth();
    }
    void BuildExplorerChrome(){
        var titleRow=new Grid{Height=40,Background=chrome,Padding=new Thickness(6,0,8,0)};
        titleRow.ColumnDefinitions.Add(new(){Width=new GridLength(1,GridUnitType.Star)});titleRow.ColumnDefinitions.Add(new(){Width=GridLength.Auto});
        titleRow.Children.Add(BuildTabs());
        var views=explorerViews;
        views.Children.Add(FlatButton("List view","\uE8FD",()=>SetView(false),true));views.Children.Add(FlatButton("Icon view","\uE80A",()=>SetView(true),true));
        views.Children.Add(FlatButton("Quick look","\uE8A7",Preview,true));views.Children.Add(FlatButton("Columns","\uE9E9",Columns,true));
        Grid.SetColumn(views,1);titleRow.Children.Add(views);explorerChrome.Children.Add(Ruled(titleRow));
        var commands=new Grid{Height=40,Padding=new Thickness(12,0,8,0)};
        commands.ColumnDefinitions.Add(new(){Width=new GridLength(1,GridUnitType.Star)});commands.ColumnDefinitions.Add(new(){Width=GridLength.Auto});
        var commandItems=new StackPanel{Orientation=Orientation.Horizontal,Spacing=10,VerticalAlignment=VerticalAlignment.Center};
        sortButton=FlatButton("Sort: Name","\uE8CB",SortFiles);commandItems.Children.Add(sortButton);
        filterButton=FlatButton("Filter","\uE71C",FilterFiles);commandItems.Children.Add(filterButton);columnsButton=FlatButton("View settings","\uE9E9",Columns);commandItems.Children.Add(columnsButton);

        var more=FlatButton("More actions","\uE712",()=>Task.CompletedTask,true);var menu=new MenuFlyout();
        foreach(var (text,action) in new(string,Func<Task>)[]{("Open",OpenFile),("Quick look",Preview),("Properties & comments",Properties),("Open folder in new tab",OpenFolderTab),("Rename",Rename),("Copy files",CopyFiles),("Paste files",PasteFiles),("Quick Search",()=>{ToggleQuick();return Task.CompletedTask;}),("Save current view",SaveView),("Update index",async()=>{await api.Post("sync");message.Text="Indexing started. Word search remains available.";})}){
            var item=new MenuFlyoutItem{Text=text};item.Click+=async(_,_)=>await Guard(action);menu.Items.Add(item);
        }
        more.Flyout=menu;commandItems.Children.Add(more);batchActions=FlatButton("Select files","\uE73A",BatchMenu);batchActions.IsEnabled=false;commandItems.Children.Add(batchActions);commands.Children.Add(commandItems);
        var refresh=FlatButton("Refresh","\uE72C",RefreshCurrent,true);Grid.SetColumn(refresh,1);commands.Children.Add(refresh);explorerChrome.Children.Add(Ruled(commands));
        var address=new Grid{Height=40,Padding=new Thickness(8,5,10,5),ColumnSpacing=8};
        address.ColumnDefinitions.Add(new(){Width=GridLength.Auto});address.ColumnDefinitions.Add(new(){Width=new GridLength(1,GridUnitType.Star)});
        var arrows=new StackPanel{Orientation=Orientation.Horizontal};
        backButton=FlatButton("Back","\uE76B",Back,true);forwardButton=FlatButton("Forward","\uE76C",Forward,true);upButton=FlatButton("Up","\uE70E",Up,true);
        arrows.Children.Add(backButton);arrows.Children.Add(forwardButton);arrows.Children.Add(upButton);address.Children.Add(arrows);
        breadcrumb.FontSize=11;breadcrumb.Text="Retriever  ›  All files";breadcrumb.TextTrimming=TextTrimming.CharacterEllipsis;
        var folder=Glyph("\uE8B7",13);folder.Foreground=accent;
        var pathGrid=new Grid{ColumnSpacing=10,Padding=new Thickness(10,0,10,0)};pathGrid.ColumnDefinitions.Add(new(){Width=GridLength.Auto});pathGrid.ColumnDefinitions.Add(new(){Width=new GridLength(1,GridUnitType.Star)});
        pathGrid.Children.Add(folder);Grid.SetColumn(breadcrumb,1);pathGrid.Children.Add(breadcrumb);
        var pathBorder=new Border{Child=pathGrid,Background=chrome,BorderBrush=line,BorderThickness=new Thickness(1),CornerRadius=new CornerRadius(3)};Grid.SetColumn(pathBorder,1);address.Children.Add(pathBorder);explorerChrome.Children.Add(Ruled(address));
        columnHeader.Height=34;columnHeader.Padding=new Thickness(20,0,20,0);headerScroll.Content=columnHeader;explorerChrome.Children.Add(Ruled(headerScroll));
        tableScroll.Content=files;tableScroll.SizeChanged+=(_,_)=>SizeFileTable();tableScroll.ViewChanged+=(_,_)=>headerScroll.ChangeView(tableScroll.HorizontalOffset,null,null,true);headerScroll.ViewChanged+=(_,_)=>{if(Math.Abs(tableScroll.HorizontalOffset-headerScroll.HorizontalOffset)>1)tableScroll.ChangeView(headerScroll.HorizontalOffset,null,null,true);};
        previousPage=FlatButton("Previous page","",()=>PageAt(offset-200));nextPage=FlatButton("Next page","",()=>PageAt(nextOffset??offset));previousPage.IsEnabled=false;nextPage.IsEnabled=false;
        paging.Margin=new Thickness(12,2,8,2);pageLabel.FontSize=10;
        paging.ColumnDefinitions.Add(new(){Width=new GridLength(1,GridUnitType.Star)});
        paging.ColumnDefinitions.Add(new(){Width=GridLength.Auto});paging.ColumnDefinitions.Add(new(){Width=GridLength.Auto});paging.ColumnDefinitions.Add(new(){Width=GridLength.Auto});
        var hint=Label("Double-click to open · Space to preview",10);hint.Opacity=.45;hint.Margin=new Thickness(8,0,12,0);Grid.SetColumn(hint,1);paging.Children.Add(hint);
        paging.Children.Add(pageLabel);Grid.SetColumn(previousPage,2);paging.Children.Add(previousPage);Grid.SetColumn(nextPage,3);paging.Children.Add(nextPage);
    }
    void BuildFileColumns(){
        columnHeader.Children.Clear();columnHeader.ColumnDefinitions.Clear();
        var columns=new List<(string Key,string Name,string Binding,string Width)>{("name","Name","Name","*")};
        foreach(var c in new[]{("modified","Date modified","Modified","145"),("type","Type","TypeLabel","135"),("size","Size","Size","84"),("duration","Length","Duration","90"),("priority","Priority","Priority","80"),("tags","Tags","Tags","120"),("status","Status","Status","90"),("device","Device","DeviceLabel","108")})if(visibleColumns.Contains(c.Item1))columns.Add(c);
        foreach(var d in propertyDefinitions)if(d!=null&&visibleColumns.Contains("prop:"+d["id"]))columns.Add(("prop:"+d["id"],d["name"]!.ToString(),"PropertyLabels["+d["id"]+"]","140"));
        minimumTableWidth=250+64+columns.Skip(1).Sum(c=>double.Parse(c.Width));SizeFileTable();
        columnHeader.ColumnDefinitions.Add(new(){Width=new GridLength(30)});Grid.SetColumn(selectPage,0);columnHeader.Children.Add(selectPage);
        var definitions="<ColumnDefinition Width='30'/>";var cells="<CheckBox Grid.Column='0' Tag='select-file' IsChecked='{Binding IsSelected,Mode=OneWay}' Width='24' MinWidth='24' MinHeight='24' Padding='0' HorizontalAlignment='Left' VerticalAlignment='Center' AutomationProperties.Name='Select file'/>";int index=1;
        foreach(var c in columns){
            columnHeader.ColumnDefinitions.Add(new(){Width=c.Width=="*"?new GridLength(1,GridUnitType.Star):new GridLength(double.Parse(c.Width))});
            var b=FlatButton(c.Name,c.Key=="name"?"\uE8D2":c.Key=="modified"?"\uE787":c.Key=="tags"?"\uE8EC":c.Key=="priority"?"\uE735":"\uE8EF",()=>Task.CompletedTask);b.Click+=async(_,_)=>await Guard(()=>ColumnSettings(c.Key,c.Name,b));b.HorizontalAlignment=HorizontalAlignment.Stretch;b.HorizontalContentAlignment=c.Key=="size"?HorizontalAlignment.Right:HorizontalAlignment.Left;b.Padding=new Thickness(8,0,8,0);
            b.Tag=c.Key;b.FontSize=11;b.Opacity=.75;b.BorderBrush=line;b.BorderThickness=new Thickness(0,0,1,0);if(b.Content is StackPanel heading){foreach(var text in heading.Children.OfType<TextBlock>())text.FontSize=11;if(sort==c.Key){heading.Children.Add(Glyph(direction=="desc"?"\uE74B":"\uE74A",9));}}Grid.SetColumn(b,index);columnHeader.Children.Add(b);definitions+=$"<ColumnDefinition Width='{c.Width}'/>";
            cells+=c.Key=="name"?"<Grid Grid.Column='1' ColumnSpacing='9' ToolTipService.ToolTip='{Binding Detail}'><Grid.ColumnDefinitions><ColumnDefinition Width='20'/><ColumnDefinition Width='*'/></Grid.ColumnDefinitions><Grid><FontIcon Glyph='{Binding Glyph}' FontSize='16' Foreground='#E5BD4C'/><Image Source='{Binding Icon}' Width='18' Height='18'/></Grid><TextBlock Grid.Column='1' Text='{Binding Name}' TextTrimming='CharacterEllipsis' VerticalAlignment='Center'/></Grid>":$"<TextBlock Grid.Column='{index}' Text='{{Binding {c.Binding}}}' TextTrimming='CharacterEllipsis' VerticalAlignment='Center' Opacity='.72' Margin='8,0,8,0'"+(c.Key=="size"?" HorizontalAlignment='Right'":"")+"/>";
            if(c.Key is "tags" or "priority" or "status" || c.Key.StartsWith("prop:",StringComparison.Ordinal)){
                var needle=$"<TextBlock Grid.Column='{index}'";var start=cells.LastIndexOf(needle,StringComparison.Ordinal);if(start>=0)cells=cells[..start]+$"<Button Grid.Column='{index}' Tag='cell:{c.Key}' Content='{{Binding {c.Binding}}}' HorizontalAlignment='Stretch' HorizontalContentAlignment='Left' Padding='8,2' MinWidth='0' MinHeight='28' BorderThickness='0' Background='Transparent' FontSize='11' CornerRadius='4'/>";
            }
            index++;
        }
        columnHeader.ColumnDefinitions.Add(new(){Width=new GridLength(32)});var add=FlatButton("Add property","\uE710",CreatePropertyPopover,true);Grid.SetColumn(add,index);columnHeader.Children.Add(add);definitions+="<ColumnDefinition Width='32'/>";
        files.ItemTemplate=(DataTemplate)XamlReader.Load("<DataTemplate xmlns='http://schemas.microsoft.com/winfx/2006/xaml/presentation'><Grid Height='32' Padding='8,0,0,0'><Grid.ColumnDefinitions>"+definitions+"</Grid.ColumnDefinitions>"+cells+"</Grid></DataTemplate>");
        var style=new Style(typeof(ListViewItem));style.Setters.Add(new Setter(Control.HorizontalContentAlignmentProperty,HorizontalAlignment.Stretch));style.Setters.Add(new Setter(Control.PaddingProperty,new Thickness(4,0,4,0)));style.Setters.Add(new Setter(FrameworkElement.MinHeightProperty,31d));style.Setters.Add(new Setter(Control.FontSizeProperty,12d));files.ItemContainerStyle=style;
        files.Padding=new Thickness(8,0,16,0);files.BorderThickness=new Thickness(0);
    }
    void UpdateSidebarWidth(){explorerBody.ColumnDefinitions[0].Width=new GridLength(!quick&&sidebarVisible?210:0);sidebar!.Visibility=!quick&&sidebarVisible?Visibility.Visible:Visibility.Collapsed;}
    void SelectNavigation(string name){selectedNavigation=name;foreach(var pair in navButtons)pair.Value.Background=pair.Key==name?selection:new SolidColorBrush(Microsoft.UI.Colors.Transparent);}
    void AddNav(string name,string glyph,string key,Func<Task> action,double indent=0){
        var b=FlatButton(name,glyph,action);b.HorizontalAlignment=HorizontalAlignment.Stretch;b.HorizontalContentAlignment=HorizontalAlignment.Left;b.Margin=new Thickness(4+indent,0,4,1);
        navButtons[key]=b;sidebarItems.Children.Add(b);
    }
    void Section(string title,Func<Task>? add=null){
        var row=new Grid{Margin=new Thickness(10,20,10,8)};row.ColumnDefinitions.Add(new(){Width=new GridLength(1,GridUnitType.Star)});row.ColumnDefinitions.Add(new(){Width=GridLength.Auto});
        var text=Label(title,10);text.Opacity=.65;row.Children.Add(text);
        if(add!=null){var b=FlatButton("Add "+title.ToLowerInvariant(),"\uE710",add,true);b.MinHeight=20;b.Padding=new Thickness(4,0,4,0);Grid.SetColumn(b,1);row.Children.Add(b);}
        sidebarItems.Children.Add(row);
    }
    async Task RefreshSidebar(){
        RestoreFolderTree();sidebarItems.Children.Clear();navButtons.Clear();Section("Quick access");
        AddNav("All files","\uE8A5","files",()=>Navigate("files"));AddNav("Favorites","\uE734","favorites",()=>Navigate("favorites"));
        BuildUtilities();
        Section("This PC & devices",AddRemoteDevice);
        sidebarItems.Children.Add(folderTree);await BuildFolderTree();
        var workspace=await api.Get("workspace");
        foreach(var (kind,title) in new[]{("view","Saved views"),("share","Shared folders")}){
            Section(title,kind=="view"?SaveView:null);var views=(workspace["views"]?.AsArray()??[]).Where(v=>v?["kind"]?.ToString()==kind).ToArray();
            if(views.Length==0){var empty=Label("No "+title.ToLowerInvariant(),10);empty.Opacity=.45;empty.Margin=new Thickness(10,0,10,4);sidebarItems.Children.Add(empty);}
            foreach(var v in views){if(v==null)continue;var value=v.DeepClone();AddNav(value["name"]?.ToString()??"Saved view","\uE8A1","view:"+value["id"],()=>OpenView(value));}
        }
        SelectNavigation(selectedNavigation);
    }
    string DeviceName(string id)=>config?["devices"]?.AsArray().FirstOrDefault(d=>d?["id"]?.ToString()==id)?["label"]?.ToString()??id;
    FileRow Row(JsonNode item)=>new(item,DeviceName(item["device"]?.ToString()??""));
    void UpdateChrome(){
        foreach(var button in columnHeader.Children.OfType<Button>())if(button.Tag is string key&&button.Content is StackPanel heading){while(heading.Children.Count>2)heading.Children.RemoveAt(2);if(key==sort&&string.IsNullOrWhiteSpace(search.Text))heading.Children.Add(Glyph(direction=="desc"?"\uE74B":"\uE74A",9));}
        sectionTitle.Text=search.Text.Trim().Length>0?"Search results":currentTitle;
        breadcrumb.Text="Retriever  ›  "+(location["path"]?.ToString()??sectionTitle.Text);
        var ext=location["filters"]?["rules"]?.AsArray().FirstOrDefault(r=>r?["field"]?.ToString()=="extension")?["value"]?.ToString();
        if(filterButton?.Content is StackPanel fp && fp.Children.Last() is TextBlock ft)ft.Text=ext==null?"Filter":"Filter: "+ext.TrimStart('.').ToUpperInvariant();
        backButton!.IsEnabled=backStack.Count>0;forwardButton!.IsEnabled=forwardStack.Count>0;
        upButton!.IsEnabled=location["path"]!=null&&location["device"]!=null&&!IsRoot(location["device"]!.ToString(),location["path"]!.ToString());
        PersistTabs();
        headerScroll.Visibility=gridMode?Visibility.Collapsed:Visibility.Visible;
        emptyLabel.Text=quick&&search.Text.Trim().Length==0?"Start typing to search your files.":search.Text.Trim().Length>0?"No matching files. Try a filename or a phrase from the document.":total>0?"":route=="browse"?"This folder has no visible items.":"No files in this view. Browse a device, or choose crawl folders in Disk Assistant.";emptyLabel.Visibility=rows.Count==0?Visibility.Visible:Visibility.Collapsed;
        if(sortButton?.Content is StackPanel p && p.Children.Last() is TextBlock t)t.Text="Sort: "+(sort=="modified"?"Date modified":char.ToUpper(sort[0])+sort[1..]);
    }
    bool IsRoot(string device,string path){if(location["filesystem"]?.GetValue<bool>()==true){var trimmed=path.Replace('\\','/').TrimEnd('/');return trimmed.Length==0||(trimmed.Length==2&&trimmed[1]==':');}string Normalize(string s)=>s.Replace('\\','/').TrimEnd('/');return config?["devices"]?.AsArray().FirstOrDefault(d=>d?["id"]?.ToString()==device)?["roots"]?.AsArray().Any(r=>string.Equals(Normalize(r!.ToString()),Normalize(path),StringComparison.OrdinalIgnoreCase))==true;}
    Task RefreshCurrent()=>search.Text.Trim().Length>0?Search(true):quick?EmptySearch():LoadFiles();
    Task SetView(bool iconsView){if(gridMode!=iconsView)return ToggleView();return Task.CompletedTask;}
    async Task SortBy(string key){if(key=="device")return;if(key=="priority"&&route=="browse"){message.Text="Priority sorting is available in All files and saved collections.";return;}if(search.Text.Trim().Length>0){message.Text="Search results are ranked by relevance. Clear the search to sort files.";return;}direction=sort==key&&direction=="asc"?"desc":"asc";sort=key;await PageAt(0);}
    async Task Forward(){if(forwardStack.TryPop(out var next)){backStack.Push((route,location.DeepClone().AsObject(),currentTitle,selectedNavigation,search.Text));route=next.Route;location=next.Location;currentTitle=next.Title;SelectNavigation(next.Nav);offset=0;search.Text=next.Query;if(next.Query.Length>0)await Search(true);else await LoadFiles();}}
    Task Columns()=>ViewSettings();
    Task FilterFiles()=>SmartFilters();
    JsonObject SavedViewBody()=>new(){["query"]=search.Text,["route"]=route,["location"]=route=="browse"?location.DeepClone():null,["filters"]=location["filters"]?.DeepClone()??new JsonObject{["match"]="all",["rules"]=new JsonArray()}};
    Task SaveView(){
        if(search.Text.Trim().Length>0){message.Text="Pin this tab to keep your search. Clear search and apply filters to save a collection.";return Task.CompletedTask;}
        var name=new TextBox{PlaceholderText="For example, Project documents",MaxLength=60};var body=new StackPanel{Spacing=10};body.Children.Add(name);
        var error=new TextBlock{FontSize=11,TextWrapping=TextWrapping.Wrap,Foreground=Brush(0xe5a088)};body.Children.Add(error);
        body.Children.Add(FlatButton("Save view","\uE73E",async()=>{try{if(string.IsNullOrWhiteSpace(name.Text))throw new InvalidOperationException("Enter a name for this view.");await api.Post("workspace/view",new JsonObject{["name"]=name.Text.Trim(),["kind"]="view",["body"]=SavedViewBody()});activePopover?.Hide();await RefreshSidebar();}catch(Exception ex){error.Text=ex.Message;}}));
        Popover("Save current view",body,width:280);return Task.CompletedTask;
    }
    async Task OpenView(JsonNode view){var body=view["body"];if(view["kind"]?.ToString()=="share"){await Browse(body?["device"]?.ToString()??"",body?["path"]?.ToString()??"");return;}if(body?["route"]?.ToString()=="browse"){await Browse(body["location"]?["device"]?.ToString()??"",body["location"]?["path"]?.ToString()??"");return;}typing?.Cancel();route="library";offset=0;location=new JsonObject{["filters"]=body?["filters"]?.DeepClone()};currentTitle=view["name"]?.ToString()??"Saved view";SelectNavigation("view:"+view["id"]);var query=body?["query"]?.ToString()??"";if(query.Length>0){search.Text=query;await Search(true);}else await LoadFiles();}
    Task ToggleTheme(){darkTheme=!darkTheme;root.RequestedTheme=darkTheme?ElementTheme.Dark:ElementTheme.Light;surface.Color=Brush(darkTheme?0x1f1f1fu:0xfafafau).Color;chrome.Color=Brush(darkTheme?0x242424u:0xf2f2f2u).Color;line.Color=Brush(darkTheme?0x383838u:0xdededeu).Color;selection.Color=Brush(darkTheme?0x443d29u:0xffedb3u).Color;hover.Color=Brush(darkTheme?0x353535u:0xe8e8e8u).Color;pressed.Color=Brush(darkTheme?0x454545u:0xdadadau).Color;rowHover.Color=Brush(darkTheme?0x2a2a2au:0xeeeeeeu).Color;selectedHover.Color=Brush(darkTheme?0x51472du:0xffe69bu).Color;AppWindow.TitleBar.ButtonForegroundColor=darkTheme?Microsoft.UI.Colors.LightGray:Microsoft.UI.Colors.DimGray;return Task.CompletedTask;}
}
