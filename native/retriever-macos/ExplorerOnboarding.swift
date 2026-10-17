import SwiftUI
import AppKit

@MainActor extension WorkspaceModel {
    func reviewCrawlScope(_ path:String) async {await run {self.crawlSetupPlan=try await self.request("crawl/preview",body:["device":self.localDevice,"root":path]);self.showingExplorerSetup=true}}
    func saveCrawlScope(start:Bool) async throws {
        guard let path=crawlSetupPlan["root"] as? String,let expected=crawlSetupPlan["revision"] as? String else{return}
        var candidate=configuration;var list=candidate["devices"] as? [[String:Any]] ?? []
        guard let index=list.firstIndex(where:{$0["id"] as? String==localDevice}) else{return}
        var roots=list[index]["roots"] as? [String] ?? [];if !roots.contains(path){roots.append(path)};list[index]["roots"]=roots;candidate["devices"]=list;candidate["configured"]=true
        _=try await request("config",body:candidate,revision:expected);try await refresh()
        try Data("{\"version\":1,\"choice\":\"search\"}".utf8).write(to:state.appendingPathComponent("onboarding.json"),options:.atomic)
        showingExplorerSetup=false;crawlSetupPlan=[:]
        if start {_=try await request("crawl/run",body:["device":localDevice,"root":path,"index_only":true]);await navigate("crawl")}
    }
}
struct ExplorerSetupPopover:View {
    @ObservedObject var model:WorkspaceModel
    @State private var feedback=""
    @State private var saving=false
    var body:some View {ScrollView{VStack(alignment:.leading,spacing:12){
        if let path=model.crawlSetupPlan["root"] as? String {
            Text(model.crawlSetupPlan["whole_volume"] as? Bool==true ? "Review this drive":"Review this folder").font(.headline)
            Text(path).textSelection(.enabled);Text("Always excluded").font(.caption).foregroundStyle(.secondary)
            ForEach(model.crawlSetupPlan["protections"] as? [String] ?? [],id:\.self){Text("· "+$0).font(.caption).foregroundStyle(.secondary)}
            Text("The first crawl builds local word search. Model analysis and recurring schedules are separate choices. Large scopes may finish with partial coverage; progress reports show what was skipped.")
            Text("Archive · \(model.crawlSetupPlan["archive_path"] as? String ?? "")").font(.caption2).foregroundStyle(.secondary)
            Button("Save & start local crawl"){save(true)}.disabled(saving);Button("Save for later"){save(false)}.disabled(saving)
        }else{
            Text("Your folders. Your search.").font(.headline)
            Text("Expand a drive in the sidebar to browse immediately. Smart search builds a private index only for folders or drives you choose.").foregroundStyle(.secondary)
            Button("Set up smart search"){Task{await model.includeFolder()}}.disabled(!model.canUseLocalFiles)
            Button("Continue browsing"){do{try Data("{\"version\":1,\"choice\":\"browse\"}".utf8).write(to:model.state.appendingPathComponent("onboarding.json"),options:.atomic);model.showingExplorerSetup=false}catch{feedback=error.localizedDescription}}
            Text("You can revisit setup in Disk Assistant. Crawling reads your files; generated data stays in Retriever's private archive.").font(.caption).foregroundStyle(.secondary)
        }
        if !feedback.isEmpty{Text(feedback).font(.caption).foregroundStyle(.red)}
        Button("Close"){model.showingExplorerSetup=false}.buttonStyle(.borderless)
    }.padding(16)}.frame(width:340).frame(maxHeight:520)}
    private func save(_ start:Bool){Task{saving=true;defer{saving=false};do{try await model.saveCrawlScope(start:start)}catch{feedback=error.localizedDescription}}}
}
