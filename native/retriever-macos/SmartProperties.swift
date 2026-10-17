import SwiftUI
import AppKit

@MainActor extension WorkspaceModel {
    func saveTypedProperties(owner:String,tags:[String],status:String,priority:String,starred:Bool,values:[String:Any],revision:Int) async throws {
        guard let file=editingFile else{return};var properties=values;properties["priority"]=priority
        metadata=try await request("workspace/metadata",body:["scope":"filesystem","device":file.device,"path":file.path,"revision":revision,"owner":owner,"tags":tags,"status":status,"starred":starred,"properties":properties])
        showingInspector=false
        if query.isEmpty {try await loadFiles()}else {await submitSearch()}
        message="Properties saved"
    }
    func createProperty(name:String,kind:String,options:[String]) async throws {
        _ = try await request("workspace/property",body:["name":name,"kind":kind,"options":options])
        propertyDefinitions=try await request("workspace/properties")["properties"] as? [[String:Any]] ?? []
    }
    func collectionFilters(_ rules:[[String:Any]]) async {
        typing?.cancel();sequence+=1;query="";section="files";browseRoute="library";offset=0;title=rules.isEmpty ? "All files":"Filtered files"
        location=["filters":["match":"all","rules":rules]];await run {try await self.loadFiles()}
    }
}

struct TagEditor:View {
    @Binding var tags:[String]
    var options:[String]=[]
    @State private var open=false
    var body:some View {
        Button{open=true}label:{HStack{Text("Tags").foregroundStyle(.secondary);Spacer();Text(tags.isEmpty ? "+ Add tags":tags.joined(separator:" · ")).lineLimit(1)}}.buttonStyle(.borderless)
            .popover(isPresented:$open){ChoicePopover(selected:$tags,options:options+tags,create:{value in value})}
    }
}

struct SmartPropertyField:View {
    @ObservedObject var model:WorkspaceModel
    @State private var optionsOpen=false
    let definition:[String:Any]
    @Binding var values:[String:Any]
    var id:String {definition["id"] as? String ?? ""}
    var name:String {definition["name"] as? String ?? "Property"}
    var kind:String {definition["kind"] as? String ?? "text"}
    var options:[String] {definition["options"] as? [String] ?? []}
    var text:Binding<String> {Binding(get:{if let s=values[id] as? String{return s};if let n=values[id] as? NSNumber{return n.stringValue};return ""},set:{values[id]=$0})}
    var body:some View {
        VStack(alignment:.leading,spacing:7) {
            if kind=="checkbox" {Toggle(name,isOn:Binding(get:{values[id] as? Bool ?? false},set:{values[id]=$0}))}
            else if ["select","multi_select"].contains(kind) {
                Button{optionsOpen=true}label:{HStack{Text(name).foregroundStyle(.secondary);Spacer();Text(kind=="select" ? (values[id] as? String ?? "+ Select"):(values[id] as? [String] ?? []).joined(separator:" · ")).lineLimit(1)}}.buttonStyle(.borderless)
                    .popover(isPresented:$optionsOpen){ChoicePopover(selected:Binding(get:{kind=="select" ? (values[id] as? String).map{[$0]} ?? []:values[id] as? [String] ?? []},set:{values[id]=kind=="select" ? ($0.first ?? "") as Any:$0 as Any}),options:options,multiple:kind=="multi_select",create:{value in try await model.addPropertyOption(id,value)})}
            }else {TextField(name,text:text,prompt:Text(kind=="date" ? "YYYY-MM-DD":kind=="number" ? "A number":"Text"))}
            Text(kind.replacingOccurrences(of:"_",with:" ").capitalized).font(.system(size:10)).foregroundStyle(.tertiary)
        }.padding(.vertical,3)
    }
}

struct InspectorView:View {
    @ObservedObject var model:WorkspaceModel
    @State private var owner=""
    @State private var status=""
    @State private var priority=""
    @State private var comment=""
    @State private var error=""
    @State private var newPropertyOpen=false
    @State private var reusableTags:[String]=[]
    @State private var tags:[String]=[]
    @State private var values:[String:Any]=[:]
    @State private var starred=false
    @State private var saving=false
    @State private var posting=false
    @State private var baseRevision=0
    private func initialize(){owner=model.metadata["owner"] as? String ?? "";status=model.metadata["status"] as? String ?? "";priority=model.metadata["priority"] as? String ?? "";starred=model.metadata["starred"] as? Bool ?? false;tags=model.metadata["tags"] as? [String] ?? [];values=model.metadata["properties"] as? [String:Any] ?? [:];baseRevision=model.metadata["revision"] as? Int ?? 0}
    private func save() async {
        saving=true;defer{saving=false}
        do {var typed=values;for field in model.propertyDefinitions {guard field["kind"] as? String=="number",let id=field["id"] as? String,let raw=values[id] as? String else{continue};if raw.trimmingCharacters(in:.whitespacesAndNewlines).isEmpty{typed[id]=NSNull()}else{guard let number=Double(raw),number.isFinite else{throw NSError(domain:"Retriever",code:1,userInfo:[NSLocalizedDescriptionKey:"\(field["name"] ?? "Property") needs a number (use a decimal point)."])};typed[id]=number}}
            try await model.saveTypedProperties(owner:owner,tags:tags,status:status,priority:priority,starred:starred,values:typed,revision:baseRevision)
        }catch{self.error=error.localizedDescription}
    }
    var body:some View {
        VStack(alignment:.leading,spacing:14) {
            HStack{Image(nsImage:model.editingFile?.icon ?? NSImage()).resizable().frame(width:28,height:28);Text(model.editingFile?.name ?? "Properties").font(.title3).lineLimit(1);Spacer()}
            Text(model.editingFile?.path ?? "").font(.caption).foregroundStyle(.secondary).textSelection(.enabled).lineLimit(2)
            Divider()
            ScrollView {
                VStack(alignment:.leading,spacing:20) {
                    Form {
                        Picker("Priority",selection:$priority){Text("None").tag("");ForEach(["low","medium","high","urgent"],id:\.self){Text($0.capitalized).tag($0)}}
                        TextField("Owner",text:$owner);TextField("Status",text:$status,prompt:Text("For example, In review"));Toggle("Favorite",isOn:$starred)
                    }
                    TagEditor(tags:$tags,options:reusableTags)
                    Divider()
                    VStack(alignment:.leading,spacing:10){ForEach(model.propertyDefinitions.indices,id:\.self){index in SmartPropertyField(model:model,definition:model.propertyDefinitions[index],values:$values)}}
                    newProperty
                    Text("Property types are shared by your workspace. Values and comments stay with each file, including after a reviewed rename.").font(.caption).foregroundStyle(.secondary)
                    Divider();comments
                }.padding(.trailing,8)
            }
            if !error.isEmpty {Text(error).foregroundStyle(.red).font(.caption).textSelection(.enabled)}
            Divider()
            HStack{Button("Close"){model.showingInspector=false}.keyboardShortcut(.cancelAction);Spacer();if saving{ProgressView().controlSize(.small)};Button("Save properties"){Task{await save()}}.buttonStyle(.borderedProminent).disabled(saving)}
        }.padding(14).frame(width:340,height:510).background(Color(nsColor:.windowBackgroundColor)).tint(Color(red:0.90,green:0.74,blue:0.30)).onAppear(perform:initialize).task{reusableTags=(try? await model.request("workspace/properties")["tags"] as? [String]) ?? []}
    }
    private var newProperty:some View {
        Button("+ Add a property"){newPropertyOpen=true}.buttonStyle(.borderless).popover(isPresented:$newPropertyOpen){NewPropertyPopover(model:model)}
    }
    private var comments:some View {
        VStack(alignment:.leading,spacing:12) {
            Text("Comments").font(.headline)
            let entries=model.metadata["comments"] as? [[String:Any]] ?? []
            if entries.isEmpty {Text("No comments yet. Keep decisions and context with the file.").foregroundStyle(.secondary).font(.caption)}
            ForEach(entries.indices,id:\.self){index in let entry=entries[index];VStack(alignment:.leading,spacing:4){HStack{Text(entry["author"] as? String ?? "");if let created=entry["created"] as? Double{Text(Date(timeIntervalSince1970:created).formatted(date:.abbreviated,time:.shortened))}}.font(.caption).foregroundStyle(.secondary);Text(entry["body"] as? String ?? "").textSelection(.enabled)}.frame(maxWidth:.infinity,alignment:.leading)}
            TextField("Add a comment…",text:$comment,axis:.vertical).lineLimit(3...6)
            Button("Post comment"){Task{posting=true;defer{posting=false};do{try await model.addComment(comment.trimmingCharacters(in:.whitespacesAndNewlines));comment="";error=""}catch{self.error=error.localizedDescription}}}.disabled(posting||comment.trimmingCharacters(in:.whitespacesAndNewlines).isEmpty)
        }
    }
}

struct SmartCollectionFilters:View {
    @ObservedObject var model:WorkspaceModel
    @Environment(\.dismiss) private var dismiss
    @State private var name=""
    @State private var tag=""
    @State private var type=""
    @State private var priority=""
    @State private var status=""
    var body:some View {
        VStack(alignment:.leading,spacing:10) {
            Text("Filter · all conditions").font(.headline)
            Form {
                TextField("Name contains",text:$name)
                Picker("File type",selection:$type){Text("All types").tag("");ForEach(["pdf","docx","pptx","xlsx","txt","md","png","jpg","mp4","mkv"],id:\.self){Text($0.uppercased()).tag($0)}}
                TextField("Tag",text:$tag)
                Picker("Priority",selection:$priority){Text("Any priority").tag("");ForEach(["low","medium","high","urgent"],id:\.self){Text($0.capitalized).tag($0)}}
                TextField("Status",text:$status)
            }
            Text("All conditions must match. Save this view to create a collection that updates as file properties change.").font(.caption).foregroundStyle(.secondary)
            HStack{Button("Cancel"){dismiss()}.keyboardShortcut(.cancelAction);Spacer();Button("Apply"){var rules:[[String:Any]]=[];for (field,op,value) in [("name","contains",name),("extension","is",type),("tags","contains",tag),("priority","is",priority),("status","is",status)]{let value=value.trimmingCharacters(in:.whitespacesAndNewlines);if !value.isEmpty {rules.append(["field":field,"operator":op,"value":value])}};Task{await model.collectionFilters(rules);dismiss()}}.buttonStyle(.borderedProminent)}
        }.padding(14).frame(width:300).onAppear{for rule in (model.location["filters"] as? [String:Any])?["rules"] as? [[String:Any]] ?? [] {let value=rule["value"] as? String ?? "";switch rule["field"] as? String {case "name":name=value;case "extension":type=value;case "tags":tag=value;case "priority":priority=value;case "status":status=value;default:break}}}
    }
}
