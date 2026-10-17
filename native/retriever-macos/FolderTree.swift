import SwiftUI
import AppKit

struct UtilitiesSection:View {
    @ObservedObject var model:WorkspaceModel
    @ObservedObject var tree:SidebarFolderState
    private let utilities=[("health","Disk Health","heart.text.square"),("disk","Disk Capacity","externaldrive"),("assistant","Disk Assistant","sparkles"),("burner","Disk Burner","opticaldisc")]
    var body:some View {
        VStack(spacing:1){
            Button{withAnimation(.easeInOut(duration:0.16)){tree.utilitiesCollapsed.toggle()};tree.save(model)}label:{HStack(spacing:6){Image(systemName:tree.utilitiesCollapsed ? "chevron.right":"chevron.down");Text("Utilities");Spacer()}.font(.system(size:10)).foregroundStyle(.secondary).padding(.horizontal,10).padding(.top,20).padding(.bottom,8)}.buttonStyle(.plain).accessibilityLabel((tree.utilitiesCollapsed ? "Expand":"Collapse")+" Utilities")
            if !tree.utilitiesCollapsed {ForEach(utilities,id:\.0){key,title,symbol in
                Button{Task{await model.navigateDestination(state:state(key,title))}}label:{HStack(spacing:8){Image(systemName:symbol).frame(width:16);Text(title);Spacer()}.font(.system(size:12)).frame(height:28).padding(.horizontal,8).contentShape(Rectangle())}.buttonStyle(.plain).background(model.section==key ? Color.yellow.opacity(0.17):.clear,in:RoundedRectangle(cornerRadius:3)).padding(.horizontal,4).contextMenu{Button("Open in new tab"){Task{await model.navigateDestination(state:state(key,title),new:true)}}}
            }}
        }
    }
    private func state(_ key:String,_ title:String)->[String:Any]{["route":key,"section":key,"title":title,"query":"","location":key=="disk" ? ["root":NSHomeDirectory()]:[:]]}
}

@MainActor final class SidebarFolderState:ObservableObject {
    struct Branch {var entries:[[String:Any]]=[];var next:Int?;var loading=false;var loaded=false;var error=""}
    @Published var utilitiesCollapsed=false
    @Published var expanded=Set<String>()
    @Published var collapsed=Set<String>()
    @Published var branches:[String:Branch]=[:]
    var endpoint=""
    func restore(_ model:WorkspaceModel){
        guard endpoint != model.endpoint else{return};endpoint=model.endpoint;model.locations=[:];expanded=[];collapsed=[];branches=[:]
        let path=model.state.appendingPathComponent("folder-tree.json")
        if let data=try? Data(contentsOf:path),data.count<131072,let saved=(try? JSONSerialization.jsonObject(with:data)) as? [String:Any],saved["endpoint"] as? String==model.endpoint {utilitiesCollapsed=saved["utilities_collapsed"] as? Bool ?? false;expanded=Set((saved["expanded"] as? [String] ?? []).prefix(256));collapsed=Set(saved["collapsed"] as? [String] ?? [])}
    }
    func save(_ model:WorkspaceModel){do{let data=try JSONSerialization.data(withJSONObject:["endpoint":model.endpoint,"utilities_collapsed":utilitiesCollapsed,"expanded":Array(expanded.prefix(256)),"collapsed":Array(collapsed)]);try data.write(to:model.state.appendingPathComponent("folder-tree.json"),options:.atomic)}catch{model.message="Could not save folder expansion: \(error.localizedDescription)"}}
    func toggle(_ model:WorkspaceModel,_ device:String,_ path:String) async {let key=device+":"+path;if expanded.contains(key){expanded.remove(key)}else{expanded.insert(key)};save(model);if expanded.contains(key){await load(model,device,path)}}
    func load(_ model:WorkspaceModel,_ device:String,_ path:String,more:Bool=false,retry:Bool=false) async {
        let key=device+":"+path;var branch=branches[key] ?? Branch();guard !branch.loading && (!branch.loaded || more || retry) else{return}
        if branches[key]==nil && branches.count>=256 {branches=branches.filter{expanded.contains($0.key)||$0.value.loading};guard branches.count<256 else{expanded.remove(key);model.message="Collapse unused folders before expanding more.";return}}
        branch.loading=true;branch.error="";branches[key]=branch
        do{let result=try await model.request("explorer/children",body:["device":device,"path":path,"offset":more ? branch.next ?? 0:0]);branch.entries=(more ? branch.entries:[])+(result["entries"] as? [[String:Any]] ?? []);branch.next=result["next_offset"] as? Int;branch.loaded=true}catch{branch.error=error.localizedDescription}
        branch.loading=false;branches[key]=branch
    }
}
struct SidebarFolderTree:View {
    @ObservedObject var model:WorkspaceModel
    @ObservedObject var tree:SidebarFolderState
    private let yellow=Color(red:0.9,green:0.74,blue:0.3)
    var body:some View {
        VStack(spacing:1){ForEach(model.devices){device in
            row(device.label,"desktopcomputer",0,!tree.collapsed.contains(device.id),true,selected:false,open:{Task{await model.browseLocations(device.id);if let path=model.locations[device.id]?.first?["path"] as? String{await model.browse(device:device.id,path:path)}}},toggle:{Task{if tree.collapsed.contains(device.id){tree.collapsed.remove(device.id);await model.browseLocations(device.id)}else{tree.collapsed.insert(device.id)};tree.save(model)}})
            if !tree.collapsed.contains(device.id){
                if model.locations[device.id]==nil && device.id != model.localDevice {Button("Connect to browse"){Task{await model.browseLocations(device.id)}}.buttonStyle(.borderless).font(.caption).frame(maxWidth:.infinity,alignment:.leading).padding(.leading,24)}
                ForEach(Array((model.locations[device.id] ?? []).enumerated()),id:\.offset){_,entry in branch(device.id,entry["path"] as? String ?? "",entry["name"] as? String ?? "Folder",entry["kind"] as? String=="home" ? "house":"externaldrive",1)}
                ForEach(device.roots.filter{_ in (model.locations[device.id] ?? []).isEmpty},id:\.self){path in branch(device.id,path,URL(fileURLWithPath:path).lastPathComponent,"folder.fill",1)}
            }
        }}.task(id:model.endpoint+model.devices.map(\.id).joined(separator:",")) {tree.restore(model);if !model.localDevice.isEmpty{await model.browseLocations(model.localDevice)}}
    }
    private func row(_ name:String,_ symbol:String,_ depth:Int,_ expanded:Bool,_ expandable:Bool,selected:Bool,open:@escaping()->Void,toggle:@escaping()->Void)->some View {
        HStack(spacing:2){Button(action:toggle){Image(systemName:expanded ? "chevron.down":"chevron.right").font(.system(size:9)).frame(width:16,height:28)}.disabled(!expandable).accessibilityLabel((expanded ? "Collapse ":"Expand ")+name)
            Button(action:open){HStack(spacing:6){if symbol=="folder.fill"{Image(nsImage:NSWorkspace.shared.icon(for:.folder)).resizable().frame(width:16,height:16)}else{Image(systemName:symbol).frame(width:16).foregroundStyle(.secondary)};Text(name).lineLimit(1);Spacer(minLength:0)}.frame(maxWidth:.infinity,minHeight:28).contentShape(Rectangle())}.accessibilityLabel(name)
        }.buttonStyle(.plain).padding(.leading,4+CGFloat(min(depth,12))*12).padding(.trailing,4).background(selected ? yellow.opacity(0.17):.clear,in:RoundedRectangle(cornerRadius:3))
    }
    private func branch(_ device:String,_ path:String,_ name:String,_ symbol:String,_ depth:Int)->AnyView {
        let key=device+":"+path;let expanded=tree.expanded.contains(key);let node=tree.branches[key]
        return AnyView(VStack(spacing:1){row(name,symbol,depth,expanded,node?.loaded != true || node?.entries.isEmpty == false || node?.error.isEmpty == false,selected:model.location["path"] as? String==path && model.location["device"] as? String==device,open:{Task{await model.browse(device:device,path:path)}},toggle:{Task{await tree.toggle(model,device,path)}}).help(path).contextMenu{Button("Open in new tab"){Task{await model.newTab(state:model.folderTabState(device,path))}};Button("Refresh folders"){Task{await tree.load(model,device,path,retry:true)}}}
            if expanded {
                if node==nil || node?.loading==true {note("Loading folders…",depth+1).task{await tree.load(model,device,path)}}
                else if let node,!node.error.isEmpty {Button("Retry loading folders"){Task{await tree.load(model,device,path,retry:true)}}.buttonStyle(.borderless).font(.caption).help(node.error).padding(.leading,24+CGFloat(depth)*12)}
                else if depth>=16 {note("Open in a tab to go deeper",depth+1)}
                else if let node {ForEach(Array(node.entries.enumerated()),id:\.offset){_,entry in branch(device,entry["path"] as? String ?? "",entry["name"] as? String ?? "Folder","folder.fill",depth+1)}
                    if node.loaded && node.entries.isEmpty {note("No subfolders",depth+1)}
                    if node.next != nil && node.entries.count<1000 {Button("More folders"){Task{await tree.load(model,device,path,more:true)}}.buttonStyle(.borderless).font(.caption).padding(.leading,24+CGFloat(depth)*12)}else if node.next != nil {note("Open in a tab for more folders",depth+1)}
                }
            }
        })
    }
    private func note(_ text:String,_ depth:Int)->some View {Text(text).font(.system(size:10)).foregroundStyle(.secondary).frame(maxWidth:.infinity,alignment:.leading).padding(.leading,24+CGFloat(depth)*12).padding(.vertical,4)}
}
