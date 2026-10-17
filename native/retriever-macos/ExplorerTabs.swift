import SwiftUI
import AppKit

struct ExplorerTab: Identifiable {
    var id=UUID().uuidString
    var pinned=false
    var state:[String:Any]=["route":"library","location":[:],"title":"All files","section":"files","query":"","sort":"name","descending":false]
}

@MainActor extension WorkspaceModel {
    var tabSessionURL:URL {state.appendingPathComponent("explorer-tabs.json")}
    func historyJSON(_ entries:[(String,[String:Any],String,String)])->[[String:Any]] {
        entries.suffix(20).map{["route":$0.0,"location":$0.1,"title":$0.2,"query":$0.3]}
    }
    func readTabHistory(_ value:Any?)->[(String,[String:Any],String,String)] {
        (value as? [[String:Any]] ?? []).suffix(20).map{(["browse","disk","health","assistant","burner"].contains($0["route"] as? String ?? "") ? $0["route"] as! String:"library",$0["location"] as? [String:Any] ?? [:],$0["title"] as? String ?? "All files",$0["query"] as? String ?? "")}
    }
    func captureTab() {
        guard !quickSearch,let index=tabs.firstIndex(where:{$0.id==activeTabID}) else{return}
        tabs[index].state=["route":browseRoute,"location":location,"title":title,"section":["files","favorites","search","disk","health","assistant","burner"].contains(section) ? section:"files","query":query,"sort":sortKey,"descending":descending,"offset":offset,"grid":gridMode,"sum_lengths":sumLengths,"search_mode":searchMode,"columns":Array(visibleColumns),"selection":Array(selection),"back":historyJSON(backStack),"forward":historyJSON(forwardStack)]
    }
    func rememberTabs() {
        guard tabsReady,!restoringTab,!quickSearch else{return};captureTab()
        do {let data=try JSONSerialization.data(withJSONObject:["version":1,"endpoint":endpoint,"active":activeTabID,"tabs":tabs.map{["id":$0.id,"pinned":$0.pinned,"state":$0.state]}]);try data.write(to:tabSessionURL,options:.atomic)}
        catch {message="Could not save tabs: "+error.localizedDescription}
    }
    func restoreTabs() async {
        guard !quickSearch else{return};restoringTab=true
        if let data=try? Data(contentsOf:tabSessionURL),data.count<=2*1024*1024 {
            do {if let saved=try JSONSerialization.jsonObject(with:data) as? [String:Any],saved["version"] as? Int==1,saved["endpoint"] as? String==endpoint,let entries=saved["tabs"] as? [[String:Any]],!entries.isEmpty {
                tabs=entries.prefix(24).map{ExplorerTab(id:$0["id"] as? String ?? UUID().uuidString,pinned:$0["pinned"] as? Bool ?? false,state:$0["state"] as? [String:Any] ?? ExplorerTab().state)}
                activeTabID=saved["active"] as? String ?? tabs[0].id
            }}catch {message="The previous tab session could not be read. Starting with All files."}
        }
        tabsReady=true;restoringTab=false
        let id=tabs.first(where:{$0.id==activeTabID})?.id ?? tabs[0].id;activeTabID="";await switchTab(id)
    }
    func switchTab(_ id:String) async {
        guard activeTabID != id,let target=tabs.first(where:{$0.id==id}) else{return}
        captureTab();typing?.cancel();sequence+=1;let generation=sequence
        restoringTab=true;activeTabID=id;let data=target.state
        browseRoute=["browse","disk","health","assistant","burner"].contains(data["route"] as? String ?? "") ? data["route"] as! String:"library";location=data["location"] as? [String:Any] ?? [:]
        title=data["title"] as? String ?? "All files";section=data["section"] as? String ?? "files";query=data["query"] as? String ?? ""
        sortKey=data["sort"] as? String ?? "name";if !["name","modified","type","size","priority","owner"].contains(sortKey){sortKey="name"}
        descending=data["descending"] as? Bool ?? false;offset=max(0,min(1_000_000,data["offset"] as? Int ?? 0));gridMode=data["grid"] as? Bool ?? false
        sumLengths=data["sum_lengths"] as? Bool ?? true;searchMode=["exact","semantic"].contains(data["search_mode"] as? String ?? "") ? data["search_mode"] as! String:"hybrid"
        visibleColumns=Set(data["columns"] as? [String] ?? ["modified","type","size","device"])
        backStack=readTabHistory(data["back"]);forwardStack=readTabHistory(data["forward"]);backCount=backStack.count;forwardCount=forwardStack.count
        files=[];selection=[];total=0;nextOffset=nil
        await run {if self.query.isEmpty {try await self.loadFiles()}else {try await self.search(full:true,captured:self.query,revision:generation)}}
        guard generation==sequence else{if activeTabID==id{restoringTab=false};return}
        selection=Set(data["selection"] as? [String] ?? []).intersection(files.map(\.id))
        // SwiftUI's query onChange is delivered while the request is awaiting.
        await Task.yield();restoringTab=false;rememberTabs()
    }
    func newTab(state:[String:Any]?=nil) async {
        guard tabs.count<24 else{message="Close a tab before opening another (24-tab limit).";return}
        captureTab();var tab=ExplorerTab();if let state {tab.state=state};tabs.append(tab);await switchTab(tab.id)
    }
    func folderTabState(_ device:String,_ path:String)->[String:Any] {
        var state=ExplorerTab().state;state["route"]="browse";state["location"]=["device":device,"path":path,"filesystem":true];state["title"]=path;return state
    }
    func destinationKey(_ state:[String:Any])->String {
        guard (state["query"] as? String ?? "").isEmpty else{return ""}
        let route=state["route"] as? String ?? "library"
        if ["disk","health","assistant","burner"].contains(route){return route}
        guard route=="browse",let loc=state["location"] as? [String:Any],let device=loc["device"] as? String,var path=loc["path"] as? String else{return ""}
        if (configuration["devices"] as? [[String:Any]] ?? []).first(where:{$0["id"] as? String==device})?["platform"] as? String=="windows" {path=path.replacingOccurrences(of:"\\",with:"/").uppercased()}
        while path.hasSuffix("/"){path.removeLast()};return "browse\n"+device+"\n"+path
    }
    func reuseDestination(_ state:[String:Any]) async -> Bool {
        captureTab();let key=destinationKey(state);guard !key.isEmpty else{return false}
        let match=tabs.first(where:{$0.id==activeTabID && destinationKey($0.state)==key}) ?? tabs.first(where:{destinationKey($0.state)==key})
        guard let match else{return false};await switchTab(match.id);return true
    }
    func navigateDestination(state:[String:Any],new:Bool=false) async {
        if new {await newTab(state:state);return};if await reuseDestination(state){return}
        guard let index=tabs.firstIndex(where:{$0.id==activeTabID}) else{await newTab(state:state);return}
        backStack.append((browseRoute,location,title,query));var target=state;target["back"]=historyJSON(backStack);target["forward"]=[] as [[String:Any]]
        tabs[index].state=target;let id=activeTabID;activeTabID="";await switchTab(id)
    }
    func closeTab(_ id:String) async {
        guard let index=tabs.firstIndex(where:{$0.id==id}),!tabs[index].pinned else{return}
        if activeTabID==id {captureTab()};var closed=tabs[index];closed.id=UUID().uuidString;closedTabs.append(closed);if closedTabs.count>10 {closedTabs.removeFirst()}
        tabs.remove(at:index);if tabs.isEmpty {tabs=[ExplorerTab()]}
        if activeTabID==id {activeTabID="";await switchTab(tabs[min(index,tabs.count-1)].id)};rememberTabs()
    }
    func duplicateTab(_ id:String) async {captureTab();if let tab=tabs.first(where:{$0.id==id}) {await newTab(state:tab.state)}}
    func reopenTab() async {guard tabs.count<24,let tab=closedTabs.popLast() else{return};await newTab(state:tab.state)}
    func cycleTab(_ delta:Int) async {guard let index=tabs.firstIndex(where:{$0.id==activeTabID}) else{return};await switchTab(tabs[(index+delta+tabs.count)%tabs.count].id)}
    func pinTab(_ id:String) {if let i=tabs.firstIndex(where:{$0.id==id}) {tabs[i].pinned.toggle();rememberTabs()}}
    func moveTab(_ id:String,before target:String) {guard id != target,let from=tabs.firstIndex(where:{$0.id==id}),let to=tabs.firstIndex(where:{$0.id==target}) else{return};let tab=tabs.remove(at:from);tabs.insert(tab,at:from<to ? to-1:to);rememberTabs()}
    func shiftTab(_ id:String,_ delta:Int) {guard let from=tabs.firstIndex(where:{$0.id==id}) else{return};let to=max(0,min(tabs.count-1,from+delta));let tab=tabs.remove(at:from);tabs.insert(tab,at:to);rememberTabs()}
    func folderInTab() async {guard let file=selected else{return};await newTab(state:folderTabState(file.device,file.kind=="folder" ? file.path:file.parentPath));if file.kind != "folder" {await revealInRetriever(file)}}
}

struct ExplorerTabStrip:View {
    @ObservedObject var model:WorkspaceModel
    @Environment(\.accessibilityReduceMotion) private var reducedMotion
    private func name(_ tab:ExplorerTab)->String {let query=tab.state["query"] as? String ?? "";if !query.isEmpty{return "Search: "+query};return (tab.state["title"] as? String ?? "All files").replacingOccurrences(of:"\\",with:"/").split(separator:"/").last.map(String.init) ?? "All files"}
    private func revealSelection(_ proxy:ScrollViewProxy) {
        withAnimation(reducedMotion ? nil:.easeOut(duration:0.16)) {
            proxy.scrollTo(model.activeTabID,anchor:.center)
        }
    }
    var body:some View {
        HStack(spacing:2) {
            ScrollViewReader {proxy in
            ScrollView(.horizontal) {HStack(spacing:3) {ForEach(model.tabs) {tab in
                HStack(spacing:7) {
                    Button {Task {await model.switchTab(tab.id)}} label:{HStack(spacing:7){Image(systemName:tab.pinned ? "pin.fill":"folder").foregroundStyle(.secondary);Text(name(tab)).lineLimit(1).frame(maxWidth:.infinity,alignment:.leading)}.frame(minWidth:70,maxWidth:145)}.help(name(tab))
                    if !tab.pinned {Button {Task {await model.closeTab(tab.id)}} label:{Image(systemName:"xmark").font(.system(size:9)).frame(width:18,height:24)}.accessibilityLabel("Close "+name(tab))}
                }.buttonStyle(.borderless).padding(.horizontal,9).frame(height:31).background(model.activeTabID==tab.id ? Color.primary.opacity(0.09):.clear,in:RoundedRectangle(cornerRadius:5))
                .overlay(alignment:.bottom){if model.activeTabID==tab.id {Capsule().fill(Color(red:0.90,green:0.74,blue:0.30)).frame(height:2).padding(.horizontal,8)}}
                .draggable(tab.id).dropDestination(for:String.self){items,_ in guard let id=items.first,model.tabs.contains(where:{$0.id==id}) else{return false};model.moveTab(id,before:tab.id);return true}
                .contextMenu {
                    Button("Duplicate tab"){Task {await model.duplicateTab(tab.id)}}
                    Button(tab.pinned ? "Unpin tab":"Pin tab"){model.pinTab(tab.id)}
                    Button("Move left"){model.shiftTab(tab.id,-1)};Button("Move right"){model.shiftTab(tab.id,1)}
                    Divider();Button("Close tab"){Task {await model.closeTab(tab.id)}}.disabled(tab.pinned)
                    Button("Reopen closed tab"){Task {await model.reopenTab()}}.disabled(model.closedTabs.isEmpty)
                }
                .id(tab.id)
            }}}.scrollIndicators(.hidden)
            .task(id:model.activeTabID) {
                // Wait for the selected tab to enter the layout before revealing it.
                await Task.yield()
                guard !Task.isCancelled else{return}
                revealSelection(proxy)
            }
            .onChange(of:model.tabs.map(\.id)) {_,_ in revealSelection(proxy)}
            }
            Button {Task {await model.newTab()}} label:{Image(systemName:"plus").frame(width:28,height:30)}.buttonStyle(.borderless).help("New tab · ⌘T").accessibilityLabel("New tab")
        }.animation(reducedMotion ? nil:.easeOut(duration:0.16),value:model.tabs.map(\.id)).frame(height:40)
    }
}
