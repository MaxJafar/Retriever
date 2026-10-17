import SwiftUI

@MainActor extension WorkspaceModel {
    func reloadPropertyDefinitions() async throws {propertyDefinitions=try await request("workspace/properties")["properties"] as? [[String:Any]] ?? []}
    func addPropertyOption(_ id:String,_ value:String) async throws -> String {
        try await reloadPropertyDefinitions()
        guard var definition=propertyDefinitions.first(where:{$0["id"] as? String==id}) else{throw NSError(domain:"Retriever",code:1,userInfo:[NSLocalizedDescriptionKey:"Property no longer exists."])}
        var options=definition["options"] as? [String] ?? []
        if let existing=options.first(where:{$0.localizedCaseInsensitiveCompare(value) == .orderedSame}){return existing}
        options.append(value);definition["options"]=options
        _ = try await request("workspace/property",body:definition);try await reloadPropertyDefinitions();return value
    }
    func saveCell(_ file:FileItem,metadata:[String:Any]) async throws -> [String:Any] {
        var body=metadata;body["scope"]="filesystem";body["device"]=file.device;body["path"]=file.path
        let result=try await request("workspace/metadata",body:body)
        if let i=files.firstIndex(where:{$0.id==file.id}){var raw=files[i].raw;raw["metadata"]=result;files[i]=FileItem(raw:raw)}
        message="Property saved";return result
    }
}

func tagColor(_ text:String)->Color {
    let colors=[Color(red:0.58,green:0.48,blue:0.22),Color(red:0.3,green:0.5,blue:0.44),Color(red:0.5,green:0.39,blue:0.6),Color(red:0.6,green:0.37,blue:0.42),Color(red:0.34,green:0.48,blue:0.6)]
    let sum=text.unicodeScalars.reduce(UInt(0)){($0 &* 31) &+ UInt($1.value)};return colors[Int(sum%5)]
}
struct SaveViewPopover:View {
    @ObservedObject var model:WorkspaceModel
    @Environment(\.dismiss) private var dismiss
    @State private var name=""
    @State private var error=""
    @State private var saving=false
    var body:some View {
        VStack(alignment:.leading,spacing:12){Text("Save current view").font(.headline);TextField("View name",text:$name)
            if !error.isEmpty{Text(error).font(.caption).foregroundStyle(.red)}
            Button("Save view"){Task{saving=true;defer{saving=false};do{let title=name.trimmingCharacters(in:.whitespacesAndNewlines);guard !title.isEmpty else{throw NSError(domain:"Retriever",code:1,userInfo:[NSLocalizedDescriptionKey:"Enter a name for this view."])};guard model.query.isEmpty else{throw NSError(domain:"Retriever",code:1,userInfo:[NSLocalizedDescriptionKey:"Pin this tab to keep your search. Clear search to save a collection."])}
                _ = try await model.request("workspace/view",body:["name":title,"kind":"view","body":["query":"","route":model.browseRoute,"location":model.browseRoute=="browse" ? model.location:[:],"filters":model.location["filters"] ?? ["match":"all","rules":[]]]]);try await model.refresh();dismiss()
            }catch{self.error=error.localizedDescription}}}.disabled(saving)
        }.padding(14).frame(width:280)
    }
}
struct ChoicePopover:View {
    @Binding var selected:[String]
    let options:[String]
    var multiple=true
    var create:((String)async throws->String)?
    var save:(([String])async throws->Void)?
    @State private var draft=""
    @State private var added:[String]=[]
    @State private var busy=false
    @State private var error=""
    private var choices:[String] {Array(Set(options+added+selected)).sorted{$0.localizedCaseInsensitiveCompare($1) == .orderedAscending}}
    private var matching:[String] {choices.filter{draft.isEmpty || $0.localizedCaseInsensitiveContains(draft)}}
    private func choose(_ value:String) async {
        guard !busy else{return};busy=true;defer{busy=false};var next=selected
        if next.contains(value){next.removeAll{$0==value}}else{if !multiple{next=[]};next.append(value)}
        do{try await save?(next);selected=next;error=""}catch{self.error=error.localizedDescription}
    }
    private func createOption() async {
        let value=draft.trimmingCharacters(in:.whitespacesAndNewlines);guard !value.isEmpty,!busy,let create else{return}
        busy=true;do{let saved=try await create(value);added.append(saved);draft="";busy=false;await choose(saved)}catch{self.error=error.localizedDescription;busy=false}
    }
    var body:some View {
        VStack(alignment:.leading,spacing:10){
            if !selected.isEmpty {ScrollView(.horizontal){HStack(spacing:4){ForEach(selected,id:\.self){value in Button{Task{await choose(value)}}label:{Text(value+" ×").font(.system(size:11)).padding(.horizontal,7).padding(.vertical,4).background(tagColor(value).opacity(0.35),in:RoundedRectangle(cornerRadius:4))}.buttonStyle(.plain)}}}.scrollIndicators(.hidden).frame(maxHeight:30)}
            TextField("Find an option or create one…",text:$draft).textFieldStyle(.plain).padding(7).background(.primary.opacity(0.04),in:RoundedRectangle(cornerRadius:5)).onSubmit{if let first=matching.first{Task{await choose(first)}}else{Task{await createOption()}}}
            Divider()
            ScrollView{VStack(spacing:2){ForEach(matching,id:\.self){value in Button{Task{await choose(value)}}label:{HStack{Image(systemName:selected.contains(value) ? "checkmark.square.fill":"square").foregroundStyle(.secondary);Text(value).padding(.horizontal,6).padding(.vertical,3).background(tagColor(value).opacity(0.28),in:RoundedRectangle(cornerRadius:4));Spacer()}.frame(maxWidth:.infinity,minHeight:28).contentShape(Rectangle())}.buttonStyle(.plain)}
                if create != nil && !draft.trimmingCharacters(in:.whitespacesAndNewlines).isEmpty && !choices.contains(where:{$0.localizedCaseInsensitiveCompare(draft.trimmingCharacters(in:.whitespacesAndNewlines)) == .orderedSame}) {Button("+ Create “\(draft.trimmingCharacters(in:.whitespacesAndNewlines))”"){Task{await createOption()}}.buttonStyle(.borderless).frame(maxWidth:.infinity,alignment:.leading)}
                if matching.isEmpty && draft.isEmpty{Text("Type to create your first option").foregroundStyle(.secondary)}
            }}.frame(maxHeight:210)
            if !error.isEmpty{Text(error).foregroundStyle(.red).font(.caption)}
            if save != nil {Text("Changes save as you select.").font(.system(size:10)).foregroundStyle(.secondary)}
        }.font(.system(size:12)).padding(14).frame(width:280).disabled(busy)
    }
}
struct NewPropertyPopover:View {
    @ObservedObject var model:WorkspaceModel
    @Environment(\.dismiss) private var dismiss
    @State private var name=""
    @State private var error=""
    @State private var busy=false
    var body:some View {
        VStack(alignment:.leading,spacing:10){Text("New property").font(.headline);TextField("Property name",text:$name);Text("Select a type").font(.caption).foregroundStyle(.secondary)
            ForEach(["text","number","select","multi_select","date","checkbox"],id:\.self){kind in
                Button{Task{busy=true;defer{busy=false};do{let definition=try await model.request("workspace/property",body:["name":name.trimmingCharacters(in:.whitespacesAndNewlines),"kind":kind,"options":[]]);try await model.reloadPropertyDefinitions();if let id=definition["id"] as? String{model.visibleColumns.insert("prop:"+id)};model.rememberTabs();dismiss()}catch{self.error=error.localizedDescription}}}label:{Label(kind.replacingOccurrences(of:"_",with:" ").capitalized,systemImage:kind=="date" ? "calendar":kind=="number" ? "number":kind=="checkbox" ? "checkmark.square":kind.contains("select") ? "list.bullet":"textformat").frame(maxWidth:.infinity,alignment:.leading).padding(.vertical,5)}.buttonStyle(.borderless)
            }
            if !error.isEmpty{Text(error).foregroundStyle(.red).font(.caption)}
        }.padding(14).frame(width:270).disabled(busy)
    }
}
struct FilePropertyCell:View {
    @ObservedObject var model:WorkspaceModel
    let file:FileItem
    let key:String
    @State private var open=false
    private var value:Any? {let metadata=file.raw["metadata"] as? [String:Any] ?? [:];return key.hasPrefix("prop:") ? (metadata["properties"] as? [String:Any])?[String(key.dropFirst(5))]:metadata[key]}
    private var labels:[String] {if let list=value as? [String]{return list};if let v=value,!(v is NSNull){return [String(describing:v)]};return []}
    var body:some View {
        Button{open=true}label:{HStack(spacing:3){if labels.isEmpty || labels==[""] {Text("—").foregroundStyle(.tertiary)}else{ForEach(Array(labels.prefix(2).enumerated()),id:\.offset){_,label in Text(label).font(.system(size:11)).lineLimit(1).padding(.horizontal,5).padding(.vertical,3).background(tagColor(label).opacity(0.23),in:RoundedRectangle(cornerRadius:4))};if labels.count>2{Text("+\(labels.count-2)").font(.caption)}};Spacer(minLength:0)}.frame(maxWidth:.infinity,minHeight:28).contentShape(Rectangle())}.buttonStyle(.plain)
            .popover(isPresented:$open,arrowEdge:.bottom){FilePropertyPopover(model:model,file:file,key:key)}
            .accessibilityLabel(key+" · "+labels.joined(separator:", "))
    }
}
struct FilePropertyPopover:View {
    @ObservedObject var model:WorkspaceModel
    let file:FileItem
    let key:String
    @Environment(\.dismiss) private var dismiss
    @State private var metadata:[String:Any]=[:]
    @State private var selected:[String]=[]
    @State private var options:[String]=[]
    @State private var text=""
    @State private var checked=false
    @State private var error=""
    @State private var loading=true
    @State private var saving=false
    private var id:String {key.hasPrefix("prop:") ? String(key.dropFirst(5)):key}
    private var definition:[String:Any] {model.propertyDefinitions.first{$0["id"] as? String==id} ?? [:]}
    private var kind:String {definition["kind"] as? String ?? (key=="tags" ? "multi_select":key=="priority" ? "select":"text")}
    private var title:String {definition["name"] as? String ?? key.capitalized}
    private func save(_ value:Any) async throws {
        var next=metadata;if key.hasPrefix("prop:") || key=="priority"{var properties=next["properties"] as? [String:Any] ?? [:];properties[id]=value;next["properties"]=properties}else{next[key]=value};metadata=try await model.saveCell(file,metadata:next)
    }
    var body:some View {
        VStack(alignment:.leading,spacing:10){
            Text(title).font(.headline).padding(.horizontal,14).padding(.top,14)
            if loading {ProgressView().padding(20)}
            else if ["select","multi_select"].contains(kind) {ChoicePopover(selected:$selected,options:options,multiple:kind=="multi_select",create:key=="priority" ? nil:{value in key.hasPrefix("prop:") ? try await model.addPropertyOption(id,value):value},save:{values in try await save(kind=="multi_select" ? values as Any:(values.first ?? "") as Any)})}
            else if kind=="checkbox" {Toggle(title,isOn:Binding(get:{checked},set:{value in guard !saving else{return};saving=true;Task{defer{saving=false};do{try await save(value);checked=value;error=""}catch{self.error=error.localizedDescription}}})).disabled(saving).padding(14)}
            else {VStack(alignment:.leading,spacing:12){TextField(kind=="date" ? "YYYY-MM-DD":kind=="number" ? "A number":"Empty",text:$text);Button("Save"){Task{saving=true;defer{saving=false};do{var value:Any=text.trimmingCharacters(in:.whitespacesAndNewlines);if kind=="number"{if text.isEmpty{value=NSNull()}else{guard let n=Double(text),n.isFinite else{throw NSError(domain:"Retriever",code:1,userInfo:[NSLocalizedDescriptionKey:"Enter a valid number."])};value=n}};try await save(value);dismiss()}catch{self.error=error.localizedDescription}}}.disabled(saving)}.padding(14)}
            if !error.isEmpty {Text(error).foregroundStyle(.red).font(.caption).padding(14)}
        }.frame(width:308).task{do{try await model.reloadPropertyDefinitions();metadata=try await model.request("workspace/inspect",body:["device":file.device,"path":file.path]);let data=try await model.request("workspace/properties");let value=key.hasPrefix("prop:") ? (metadata["properties"] as? [String:Any])?[id]:metadata[key];text=value as? String ?? (value as? NSNumber)?.stringValue ?? "";checked=value as? Bool ?? false;selected=value as? [String] ?? (text.isEmpty ? []:[text]);options=definition["options"] as? [String] ?? (key=="priority" ? ["low","medium","high","urgent"]:data["tags"] as? [String] ?? []);loading=false}catch{self.error=error.localizedDescription;loading=false}}
    }
}
