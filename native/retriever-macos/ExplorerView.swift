import SwiftUI
import AppKit

// Shared layout contract: native/design/EXPLORER.md.
struct WorkspaceView: View {
    @ObservedObject var model: WorkspaceModel
    var quick = false
    @FocusState private var searchFocused: Bool
    @Environment(\.accessibilityReduceMotion) private var reducedMotion
    @State private var sidebarVisible = true
    @State private var darkTheme = true
    @State private var showingSmartFilters=false
    @State private var showingColumns=false
    @State private var newProperty=false
    @State private var toolbarNewProperty=false
    @State private var savingView = false
    @State private var savingSidebarView=false
    @State private var expandedSearch=false
    @State private var showingCrawl=false
    @State private var desktopPopover=false
    @State private var connectionsPopover=false
    @State private var activityPopover=false
    @State private var batchPopover=false
    @State private var tableWidth: CGFloat = 1000
    private let yellow = Color(red:0.90,green:0.74,blue:0.30)
    private var chrome: Color {darkTheme ? Color(white:0.141) : Color(white:0.949)}
    private var canvas: Color {darkTheme ? Color(white:0.122) : Color(white:0.98)}
    private var displayTitle: String {
        model.section=="search" ? "Search results" : model.title.replacingOccurrences(of:"\\",with:"/").split(separator:"/").last.map(String.init) ?? model.title
    }
    var body: some View {
        VStack(spacing:0) {
            header.zIndex(10)
            Divider()
            HStack(spacing:0) {
                if !quick && sidebarVisible {sidebar.frame(width:210);Divider()}
                Group {
                    if model.section=="assistant" {VStack(spacing:0){ExplorerTabStrip(model:model).padding(.horizontal,12).background(chrome);Divider();DiskAssistantView(model:model)}}
                    else if model.section=="burner" {VStack(spacing:0){ExplorerTabStrip(model:model).padding(.horizontal,12).background(chrome);Divider();DiskBurnerView(model:model)}}
                    else if model.section=="settings" {SettingsView(model:model)}
                    else if model.browseRoute=="disk" && model.section=="disk" {VStack(spacing:0){ExplorerTabStrip(model:model).padding(.horizontal,12).background(chrome);Divider();DiskSpaceView(model:model)}}
                    else if model.browseRoute=="health" && model.section=="health" {VStack(spacing:0){ExplorerTabStrip(model:model).padding(.horizontal,12).background(chrome);Divider();DriveHealthView(model:model)}}
                    else if model.section=="crawl" {CrawlView(model:model)}
                    else if model.section=="activity" {ActivityView(model:model)}
                    else if model.section=="sync" {
                        VStack(alignment:.leading,spacing:16) {
                            Text("Folder sync").font(.title2)
                            Text("Imported sync baselines are retained. Mutations remain disabled until cross-device parity passes.").foregroundStyle(.secondary)
                            List(Array(model.syncPairs.enumerated()),id:\.offset) {_,item in Text(item["name"] as? String ?? "Folder pair")}
                        }.padding(24)
                    } else {fileTable}
                }.frame(maxWidth:.infinity,maxHeight:.infinity).simultaneousGesture(TapGesture().onEnded{expandedSearch=false})
            }
            Divider()
            HStack {Text(model.message).font(.system(size:10)).foregroundStyle(.secondary).lineLimit(1);Spacer();if !quick{UpdateBadge(updates:model.updates){model.showingUpdates=true}}}.padding(.horizontal,10).frame(height:23).background(chrome)
        }
        .font(.system(size:12)).tint(yellow).preferredColorScheme(darkTheme ? .dark : .light)
        .background(quick ? AnyShapeStyle(.ultraThinMaterial) : AnyShapeStyle(canvas))
        .sheet(isPresented:$model.showingPreview) {
            VStack(alignment:.leading) {HStack {Text(model.previewTitle).font(.title2);Spacer();Button("Done"){model.showingPreview=false}.keyboardShortcut(.cancelAction)}
                ScrollView {Text(model.previewText).textSelection(.enabled).frame(maxWidth:.infinity,alignment:.leading)}
            }.padding(24).frame(width:700,height:540)
        }
        .sheet(isPresented:$model.showingPlan){PlanReviewView(model:model)}
        .sheet(isPresented:$model.showingUpdates){VStack(alignment:.trailing){Button("Done"){model.showingUpdates=false}.keyboardShortcut(.cancelAction).padding([.top,.trailing]);UpdatesView(updates:model.updates,install:model.openUpdate)}}

        .popover(isPresented:$model.showingExplorerSetup){ExplorerSetupPopover(model:model)}
        .onAppear {searchFocused=true}
        .onChange(of:model.visibleColumns){_,_ in model.rememberTabs();if model.visibleColumns.contains("duration"){Task{await model.readLengths()}}}
        .onChange(of:model.gridMode){_,_ in model.rememberTabs()}
        .onReceive(NotificationCenter.default.publisher(for:Notification.Name("RetrieverFocusQuickSearch"))) {_ in if quick {searchFocused=true}}
        .background(Button(""){searchFocused=true;expandedSearch=true}.keyboardShortcut("k").hidden())
        .onExitCommand {if expandedSearch {expandedSearch=false} else if !model.query.isEmpty {model.query=""} else if quick {NSApp.keyWindow?.orderOut(nil)}}
    }
    private var header: some View {
        HStack(spacing:12) {
            HStack(spacing:9) {
                if !quick {Button {sidebarVisible.toggle()} label:{Image(systemName:"sidebar.left")}.help("Toggle sidebar")}
                if let logo=NSImage(contentsOf:Bundle.main.url(forResource:"Retriever",withExtension:"png") ?? URL(fileURLWithPath:"")) {Image(nsImage:logo).resizable().frame(width:30,height:30)}
                if !quick {Text("Retriever").font(.system(size:13,weight:.semibold));Spacer(minLength:0)}
            }.frame(width:quick ? 34 : 186)
            HStack(spacing:10) {
                Image(systemName:"magnifyingglass").font(.system(size:17)).foregroundStyle(yellow)
                TextField("Search your files, or describe what you need…",text:$model.query)
                    .textFieldStyle(.plain).font(.system(size:14)).focused($searchFocused)
                    .simultaneousGesture(TapGesture().onEnded{withAnimation(reducedMotion ? nil:.easeOut(duration:0.2)){expandedSearch=true}}).onChange(of:model.query){_,_ in model.queryChanged()}.onSubmit {Task {await model.submitSearch()}}
                Text("⌘ K").font(.system(size:10)).foregroundStyle(.secondary).padding(4).overlay(RoundedRectangle(cornerRadius:3).stroke(.quaternary))
            }.padding(.horizontal,14).frame(height:44).background(canvas,in:RoundedRectangle(cornerRadius:10)).overlay(RoundedRectangle(cornerRadius:10).stroke(.quaternary))
            .overlay(alignment:.topLeading){if expandedSearch && !quick {SearchDetailPanel(model:model){expandedSearch=false}.frame(width:min(760,max(460,tableWidth-40)),height:390).offset(y:50).transition(.opacity.combined(with:.move(edge:.top)))}}
            if !quick {
                Button {darkTheme.toggle()} label:{Image(systemName:"sun.max")}.help("Switch theme")
                Button {showingCrawl=true} label:{Image(systemName:"slider.horizontal.3")}.help("Crawling & learning").popover(isPresented:$showingCrawl){CrawlFoldersPopover(model:model)}
                Button {Task {await model.navigate("settings")}} label:{Image(systemName:"gearshape")}.help("Settings")
            }
        }.buttonStyle(.borderless).padding(.horizontal,12).frame(height:68).background(chrome)
    }
    private func nav(_ text:String,_ symbol:String,_ selected:Bool=false,compact:Bool=false,action:@escaping()->Void)->some View {
        Button(action:action) {HStack(spacing:compact ? 5 : 9){Image(systemName:symbol).frame(width:compact ? 12 : 16).foregroundStyle(symbol=="folder.fill" ? yellow : .secondary);Text(text).font(.system(size:compact ? 10 : 12)).lineLimit(1);Spacer(minLength:0)}.padding(.horizontal,compact ? 4 : 8).frame(height:28).contentShape(Rectangle())}
            .buttonStyle(.plain).background(selected ? yellow.opacity(0.17) : .clear,in:RoundedRectangle(cornerRadius:3)).padding(.horizontal,4)
    }
    private func heading(_ text:String)->some View {Text(text).font(.system(size:10)).foregroundStyle(.secondary).frame(maxWidth:.infinity,alignment:.leading).padding(.horizontal,10).padding(.top,20).padding(.bottom,6)}
    private var sidebar:some View {
        VStack(spacing:0) {
            ScrollView {
                VStack(spacing:1) {
                    heading("Quick access")
                    nav("All files","doc.on.doc",model.section=="files" && model.title=="All files"){Task {await model.navigate("files")}}
                    nav("Favorites","star",model.section=="favorites"){Task {await model.navigate("favorites")}}
                    UtilitiesSection(model:model,tree:model.folderTree)
                    heading("This PC & devices")
                    SidebarFolderTree(model:model,tree:model.folderTree)
                    ForEach(["view","share"],id:\.self) {kind in
                        HStack {heading(kind=="view" ? "Saved views" : "Shared folders")
                            if kind=="view" {Button {savingSidebarView=true} label:{Image(systemName:"plus")}.buttonStyle(.borderless).padding(.trailing,12).popover(isPresented:$savingSidebarView){SaveViewPopover(model:model)}}
                        }
                        let views=model.savedViews.filter{$0["kind"] as? String==kind}
                        if views.isEmpty {Text(kind=="view" ? "No saved views" : "No shared folders").font(.system(size:10)).foregroundStyle(.tertiary).frame(maxWidth:.infinity,alignment:.leading).padding(.horizontal,10)}
                        ForEach(Array(views.enumerated()),id:\.offset) {_,view in nav(view["name"] as? String ?? "Saved view","bookmark"){Task {await model.openView(view)}}}
                    }
                }.padding(.bottom,16)
            }
            Divider()
            VStack(spacing:4) {
                HStack(spacing:0) {nav("Folder sync","arrow.triangle.2.circlepath",compact:true){Task {await model.navigate("sync")}};nav("Desktop app","desktopcomputer",compact:true){desktopPopover=true}.popover(isPresented:$desktopPopover){UtilityPopover(model:model,kind:"Desktop app")}}
                HStack(spacing:0) {nav("Connections","network",compact:true){connectionsPopover=true}.popover(isPresented:$connectionsPopover){UtilityPopover(model:model,kind:"Connections")};nav("Activity","clock.arrow.circlepath",compact:true){activityPopover=true}.popover(isPresented:$activityPopover){UtilityPopover(model:model,kind:"Activity")}}
            }.font(.system(size:10)).padding(.vertical,8)
        }.background(chrome)
    }
    private func iconButton(_ title:String,_ symbol:String,action:@escaping()->Void)->some View {
        Button(action:action){Image(systemName:symbol).frame(width:24,height:26)}.buttonStyle(.borderless).foregroundStyle(.secondary).help(title).accessibilityLabel(title)
    }
    private func showColumn(_ key:String)->Bool {model.visibleColumns.contains(key) && (key != "device" || tableWidth>=740) && (key != "type" || tableWidth>=640)}
    private func sort(_ key:String){if key=="priority" && model.browseRoute=="browse"{model.message="Priority sorting is available in All files and saved collections.";return};guard model.section != "search" else{return};model.descending=model.sortKey==key ? !model.descending : false;model.sortKey=key;Task {await model.resort()}}
    private var customColumns:[[String:Any]] {model.propertyDefinitions.filter{model.visibleColumns.contains("prop:"+($0["id"] as? String ?? ""))}}
    private var columnMenu:some View {
        Button{showingColumns=true}label:{Label("View settings",systemImage:"slider.horizontal.3")}.buttonStyle(.borderless)
            .popover(isPresented:$showingColumns){VStack(alignment:.leading,spacing:12){Text("View settings").font(.headline)
                Picker("Layout",selection:$model.gridMode){Text("Table").tag(false);Text("Icons").tag(true)}.pickerStyle(.segmented)
                Divider();Text("Property visibility").font(.caption).foregroundStyle(.secondary)
                ScrollView{VStack(alignment:.leading,spacing:9){ForEach(["modified","type","size","device","duration","priority","tags","status"]+model.propertyDefinitions.compactMap{($0["id"] as? String).map{"prop:"+$0}},id:\.self){key in
                    Toggle(columnName(key),isOn:Binding(get:{model.visibleColumns.contains(key)},set:{if $0{model.visibleColumns.insert(key)}else{model.visibleColumns.remove(key)};model.rememberTabs()}))
                }}}.frame(maxHeight:340)
                Toggle("Sum measured video lengths",isOn:$model.sumLengths).onChange(of:model.sumLengths){_,_ in model.rememberTabs()}
                Button("+ New property"){toolbarNewProperty=true}.buttonStyle(.borderless).popover(isPresented:$toolbarNewProperty){NewPropertyPopover(model:model)}
            }.padding(14).frame(width:270)}
    }
    private func columnName(_ key:String)->String {if key=="modified"{return "Date modified"};if key=="duration"{return "Length"};if key.hasPrefix("prop:"){return model.propertyDefinitions.first{$0["id"] as? String==String(key.dropFirst(5))}?["name"] as? String ?? "Property"};return key.capitalized}
    private func columnSort(_ key:String,_ descending:Bool){guard model.query.isEmpty else{model.message="Search results are ranked by relevance. Clear search to sort files.";return};guard key != "priority" || model.browseRoute != "browse" else{model.message="Priority sorting is available in All files and saved collections.";return};model.sortKey=key;model.descending=descending;Task{await model.resort()}}
    private func tableHeading(_ key:String,_ icon:String)->some View {
        Menu {
            if ["name","modified","size","type","priority"].contains(key){Button("Sort ascending"){columnSort(key,false)};Button("Sort descending"){columnSort(key,true)}}
            if key != "name"{Button("Hide property"){model.visibleColumns.remove(key);model.rememberTabs()}}
        }label:{HStack(spacing:6){Image(systemName:icon);Text(columnName(key));if model.sortKey==key{Image(systemName:model.descending ? "arrow.down":"arrow.up").font(.system(size:8))};Spacer(minLength:0)}.foregroundStyle(.secondary).font(.system(size:11)).frame(maxWidth:.infinity,minHeight:32).contentShape(Rectangle())}.menuStyle(.borderlessButton).menuIndicator(.hidden).tint(.secondary).controlSize(.small).padding(.horizontal,6).overlay(alignment:.trailing){Divider().opacity(0.4)}
    }
    private var minimumTableWidth:CGFloat {
        let widths:[String:CGFloat]=["modified":145,"type":135,"size":84,"priority":80,"tags":120,"status":90,"device":116]
        return 318+widths.reduce(CGFloat(0)){$0+(showColumn($1.key) ? $1.value:0)}+CGFloat(customColumns.count)*140
    }
    private var fileTable:some View {
        VStack(spacing:0) {
            HStack(spacing:9) {ExplorerTabStrip(model:model)
                iconButton("List view","list.bullet"){model.gridMode=false}
                iconButton("Icon view","square.grid.2x2"){model.gridMode=true}
                iconButton("Quick Look","sidebar.right"){Task {await model.preview()}}
            }.padding(.horizontal,12).frame(height:40).background(chrome)
            Divider()
            HStack(spacing:22) {
                Menu("Sort: "+(model.sortKey=="modified" ? "Date modified" : model.sortKey.capitalized)){ForEach(["name","modified","type","size"],id:\.self){key in Button(key.capitalized){sort(key)}}}.menuStyle(.borderlessButton).fixedSize()
                Button {showingSmartFilters=true} label:{Label("Filter",systemImage:"line.3.horizontal.decrease")}.buttonStyle(.borderless).popover(isPresented:$showingSmartFilters){SmartCollectionFilters(model:model)}
                columnMenu
                iconButton("Properties & comments","text.bubble"){Task{await model.inspect()}}.popover(isPresented:$model.showingInspector){InspectorView(model:model)}
                Menu {Button("Open"){Task {await model.open()}};Button("Quick Look"){Task {await model.preview()}};Button("Properties & comments"){Task {await model.inspect()}};Button("Open folder in new tab"){Task {await model.folderInTab()}};Button("Copy files"){model.copyFiles()};Button("Paste files"){model.pasteFiles()};Divider();Button("Save current view"){savingView=true};Button("Update index"){Task {await model.index()}}} label:{Image(systemName:"ellipsis")}.menuStyle(.borderlessButton).fixedSize().popover(isPresented:$savingView){SaveViewPopover(model:model)}
                Spacer()
                iconButton("Refresh","arrow.clockwise"){if model.query.isEmpty {Task {await model.run {try await model.loadFiles()}}}else {model.queryChanged()}}
            }.tint(darkTheme ? .white : .black).padding(.horizontal,14).frame(height:40)
            Divider()
            HStack(spacing:4) {
                iconButton("Back","chevron.left"){Task {await model.back()}}.disabled(model.backCount==0)
                iconButton("Forward","chevron.right"){Task {await model.forward()}}.disabled(model.forwardCount==0)
                iconButton("Up","chevron.up"){Task {await model.up()}}.disabled(model.parentFolder==nil)
                HStack(spacing:10){Image(systemName:"folder").foregroundStyle(yellow);Text("Retriever  ›  "+model.title).lineLimit(1).truncationMode(.middle);Spacer(minLength:0)}
                    .font(.system(size:11)).padding(.horizontal,10).frame(height:28).background(chrome,in:RoundedRectangle(cornerRadius:3)).overlay(RoundedRectangle(cornerRadius:3).stroke(.quaternary))
            }.padding(.horizontal,10).frame(height:40)
            Divider()
            if model.gridMode {fileGrid}
            else {
                GeometryReader{viewport in ScrollView(.horizontal){VStack(spacing:0){
                HStack(spacing:0) {
                    Toggle("Select page",isOn:Binding(get:{!model.files.isEmpty && model.selection.count==model.files.count},set:{model.selection=$0 ? Set(model.files.map(\.id)):[]})).labelsHidden().toggleStyle(.checkbox).frame(width:30)
                    tableHeading("name","textformat").frame(maxWidth:.infinity,alignment:.leading)
                    if showColumn("modified"){tableHeading("modified","calendar").frame(width:145,alignment:.leading)}
                    if showColumn("type"){tableHeading("type","doc").frame(width:135,alignment:.leading)}
                    if showColumn("size"){tableHeading("size","number").frame(width:84,alignment:.trailing)}
                    if showColumn("duration"){tableHeading("duration","clock").frame(width:90)}
                    if showColumn("priority"){tableHeading("priority","flag").frame(width:80,alignment:.leading)}
                    if showColumn("tags"){tableHeading("tags","tag").frame(width:120,alignment:.leading)}
                    if showColumn("status"){tableHeading("status","circle.lefthalf.filled").frame(width:90,alignment:.leading)}
                    if showColumn("device"){tableHeading("device","desktopcomputer").frame(width:108,alignment:.leading).padding(.leading,8)}
                    ForEach(customColumns.indices,id:\.self){i in let d=customColumns[i];tableHeading("prop:"+(d["id"] as? String ?? ""),"list.bullet").frame(width:140)}
                    Button{newProperty=true}label:{Image(systemName:"plus").frame(width:28,height:30)}.buttonStyle(.borderless).help("Add property").popover(isPresented:$newProperty){NewPropertyPopover(model:model)}
                }.buttonStyle(.plain).font(.system(size:11)).padding(.horizontal,20).frame(height:34)
                Divider()
                ScrollViewReader {reader in List(model.files,selection:$model.selection) {file in
                    HStack(spacing:0) {
                        Toggle("Select "+file.name,isOn:Binding(get:{model.selection.contains(file.id)},set:{if $0{model.selection.insert(file.id)}else{model.selection.remove(file.id)}})).labelsHidden().toggleStyle(.checkbox).frame(width:30)
                        HStack(spacing:9){Image(nsImage:file.icon).resizable().frame(width:18,height:18);Text(file.name).lineLimit(1)}.frame(maxWidth:.infinity,alignment:.leading).help(file.detail)
                        if showColumn("modified"){Text(file.modified).frame(width:145,alignment:.leading).foregroundStyle(.secondary)}
                        if showColumn("type"){Text(file.typeLabel).frame(width:135,alignment:.leading).foregroundStyle(.secondary)}
                        if showColumn("size"){Text(file.size).frame(width:84,alignment:.trailing).foregroundStyle(.secondary)}
                        if showColumn("duration"){Text(file.length).monospacedDigit().frame(width:90).foregroundStyle(.secondary)}
                        if showColumn("priority"){FilePropertyCell(model:model,file:file,key:"priority").frame(width:80)}
                        if showColumn("tags"){FilePropertyCell(model:model,file:file,key:"tags").frame(width:120)}
                        if showColumn("status"){FilePropertyCell(model:model,file:file,key:"status").frame(width:90)}
                        if showColumn("device"){Text(model.deviceName(file.device)).frame(width:108,alignment:.leading).padding(.leading,8).foregroundStyle(.secondary)}
                        ForEach(customColumns.indices,id:\.self){i in FilePropertyCell(model:model,file:file,key:"prop:"+(customColumns[i]["id"] as? String ?? "")).frame(width:140)}
                        Color.clear.frame(width:28)
                    }.lineLimit(1).frame(height:32).overlay(alignment:.bottom){Divider().opacity(0.2)}.tag(file.id).id(file.id).listRowInsets(EdgeInsets(top:0,leading:20,bottom:0,trailing:20)).listRowSeparator(.hidden)
                }.listStyle(.plain).scrollContentBackground(.hidden).contentMargins(.horizontal,0,for:.scrollContent).environment(\.defaultMinListRowHeight,31)
                    .contextMenu {Button("Open"){Task {await model.open()}};Button("Quick Look"){Task {await model.preview()}};Button("Properties & comments"){Task {await model.inspect()}}}
                    .onTapGesture(count:2){Task {await model.open()}}
                    .onKeyPress(.return){Task {await model.open()};return .handled}
                    .onKeyPress(.space){Task {await model.preview()};return .handled}
                    .onChange(of:model.revealRevision){_,_ in if let id=model.selection.first {reader.scrollTo(id)}}
                }
                }.frame(width:max(viewport.size.width,minimumTableWidth),height:max(100,viewport.size.height-14))}}
            }
            Divider()
            HStack {
                Text("\(model.total) items · \(model.total==0 ? 0 : model.offset+1)–\(min(model.offset+model.files.count,model.total))").foregroundStyle(.secondary)
                Spacer()
                if model.gridMode {Slider(value:$model.iconSize,in:32...88).frame(width:100).accessibilityLabel("Icon size")}
                Text(model.lengthSummary).foregroundStyle(.secondary)
                Button("\(model.selection.count) selected"){batchPopover=true}.disabled(model.selection.isEmpty)
                    .popover(isPresented:Binding(get:{batchPopover || model.showingPaste},set:{if !$0 {batchPopover=false;model.showingPaste=false}})){
                        if model.showingPaste {BatchFilesPopover(model:model,operation:"copy",sourceFiles:model.copiedFiles,destination:model.location["path"] as? String ?? "",target:model.location["device"] as? String ?? "")}
                        else {BatchFilesPopover(model:model)}
                    }
                Text("Double-click to open · Space to preview").foregroundStyle(.tertiary)
                Button("Previous page"){Task {await model.page(model.offset-200)}}.disabled(model.offset==0)
                Button("Next page"){if let next=model.nextOffset {Task {await model.page(next)}}}.disabled(model.nextOffset==nil)
            }.buttonStyle(.borderless).tint(darkTheme ? .white : .black).font(.system(size:10)).padding(.horizontal,12).frame(height:28)
        }.background(quick ? AnyShapeStyle(.clear) : AnyShapeStyle(canvas))
            .background {GeometryReader {proxy in Color.clear.onAppear {tableWidth=proxy.size.width}.onChange(of:proxy.size.width){_,width in tableWidth=width}}}
    }
    private var fileGrid:some View {
        ScrollViewReader {reader in ScrollView {LazyVGrid(columns:[GridItem(.adaptive(minimum:model.iconSize+100))],spacing:16) {
            ForEach(model.files) {file in
                Button {model.select(file)} label:{
                    VStack(spacing:8){Image(nsImage:file.icon).resizable().frame(width:model.iconSize,height:model.iconSize);Text(file.name).lineLimit(2);Text(file.size).font(.caption).foregroundStyle(.secondary)}
                        .frame(maxWidth:.infinity,minHeight:model.iconSize+58).padding(8).background(model.selection.contains(file.id) ? yellow.opacity(0.18) : .clear,in:RoundedRectangle(cornerRadius:5))
                }.buttonStyle(.plain).id(file.id).simultaneousGesture(TapGesture(count:2).onEnded{model.selection=[file.id];Task {await model.open()}})
            }
        }.padding(16)}.animation(reducedMotion ? nil : .easeInOut(duration:0.15),value:model.iconSize)
            .onChange(of:model.revealRevision){_,_ in if let id=model.selection.first {reader.scrollTo(id)}}
        }
    }
}
