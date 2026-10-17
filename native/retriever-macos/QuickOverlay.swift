import SwiftUI
import AppKit
import QuartzCore

// This is a separate surface. The approved Explorer stays in ExplorerView.swift.
struct QuickOverlayView: View {
    @ObservedObject var model: WorkspaceModel
    var dismiss: () -> Void
    var showExplorer: () -> Void
    var dismissAfterOpen: (() -> Void)? = nil
    @FocusState private var searchFocused: Bool
    @State private var preview: [String:Any] = [:]
    @State private var previewError = ""
    @State private var loading = false
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    private let gold = Color(red:0.90,green:0.74,blue:0.30)
    private var selected: FileItem? { model.selected }

    var body: some View {
        VStack(spacing:0) {
            HStack(spacing:14) {
                if let logo=NSImage(contentsOf:Bundle.main.url(forResource:"Retriever",withExtension:"png") ?? URL(fileURLWithPath:"")) {
                    Image(nsImage:logo).resizable().frame(width:34,height:34).accessibilityHidden(true)
                }
                Image(systemName:"magnifyingglass").font(.system(size:21,weight:.light)).foregroundStyle(gold)
                TextField("Search files, or describe what you need…",text:$model.query)
                    .textFieldStyle(.plain).font(.system(size:20)).focused($searchFocused)
                    .accessibilityLabel("Search your files").onChange(of:model.query) {_,_ in model.queryChanged()}
                    .onSubmit {if selected != nil {openSelected()} else {Task {await model.submitSearch()}}}
                    .onKeyPress(.downArrow) {move(1);return .handled}
                    .onKeyPress(.upArrow) {move(-1);return .handled}
                if !model.query.isEmpty {Button {model.query="";searchFocused=true} label:{Image(systemName:"xmark.circle.fill")}.buttonStyle(.plain).foregroundStyle(.secondary).help("Clear search")}
                Button(action:dismiss) {Text("esc").font(.system(size:11,design:.monospaced)).padding(.horizontal,7).padding(.vertical,4)}.buttonStyle(.plain).foregroundStyle(.secondary).help("Close Quick Search")
            }.padding(.horizontal,24).frame(height:80)
            Divider().overlay(.white.opacity(0.05))
            GeometryReader {geometry in
                HStack(spacing:0) {
                    results.frame(width:geometry.size.width * 0.53)
                    Divider().overlay(.white.opacity(0.04))
                    quickLook.frame(maxWidth:.infinity,maxHeight:.infinity)
                }
            }
            Divider().overlay(.white.opacity(0.05))
            HStack(spacing:14) {
                Text(model.query.isEmpty ? "Retriever · Quick Search" : model.message).lineLimit(1).truncationMode(.tail)
                Spacer(minLength:8)
                Text("↑ ↓ Select");Text("↩ Open")
                Button("Open Retriever",action:showExplorer).buttonStyle(.plain).foregroundStyle(gold)
            }.font(.system(size:11)).foregroundStyle(.secondary).padding(.horizontal,22).frame(height:38)
        }
        .foregroundStyle(Color(white:0.94))
        .background(reduceTransparency ? Color(red:0.12,green:0.12,blue:0.12) : Color.black.opacity(0.04))
        .clipShape(RoundedRectangle(cornerRadius:24,style:.continuous))
        .overlay(RoundedRectangle(cornerRadius:24,style:.continuous).strokeBorder(LinearGradient(colors:[.white.opacity(0.25),.white.opacity(0.06)],startPoint:.topLeading,endPoint:.bottomTrailing)))
        .preferredColorScheme(.dark)
        .onExitCommand(perform:dismiss)
        .onAppear {searchFocused=true}
        .onReceive(NotificationCenter.default.publisher(for:Notification.Name("RetrieverFocusQuickSearch"))) {_ in searchFocused=true}
        .task(id:"\(model.quickActive):\(selected?.id ?? "")") {await loadPreview()}
    }
    private var results:some View {
        VStack(alignment:.leading,spacing:0) {
            HStack {Text("RESULTS").tracking(1.4);Spacer();Text("\(model.files.count)")}.font(.system(size:10,weight:.medium)).foregroundStyle(.secondary).padding(.horizontal,22).frame(height:36)
            if model.files.isEmpty {
                VStack(spacing:12) {
                    Image(systemName:model.query.isEmpty ? "sparkle.magnifyingglass" : "doc.text.magnifyingglass").font(.system(size:30,weight:.ultraLight)).foregroundStyle(gold.opacity(0.8))
                    Text(model.query.isEmpty ? "A thought away." : "No matches yet").font(.system(size:17,weight:.medium))
                    Text(model.query.isEmpty ? "Find a file by name, a phrase, or an idea." : "Try a filename or a few different words.").font(.system(size:12)).foregroundStyle(.secondary).multilineTextAlignment(.center)
                }.frame(maxWidth:.infinity,maxHeight:.infinity).padding(24)
            } else {
                ScrollViewReader {proxy in
                    ScrollView {
                        LazyVStack(spacing:4) {
                            ForEach(model.files) {file in
                                QuickResultRow(file:file,selected:selected?.id==file.id,gold:gold)
                                    .contentShape(RoundedRectangle(cornerRadius:10))
                                    .onTapGesture(count:2) {model.selection=[file.id];openSelected()}
                                    .onTapGesture {model.selection=[file.id]}
                                    .accessibilityElement(children:.combine).accessibilityAddTraits(.isButton)
                                    .accessibilityAction {model.selection=[file.id]}
                                    .id(file.id)
                            }
                        }.padding(.horizontal,10).padding(.bottom,12)
                    }.onChange(of:selected?.id) {_,id in if let id {proxy.scrollTo(id,anchor:nil)}}
                }
            }
        }.background(Color.black.opacity(0.06))
    }
    @ViewBuilder private var quickLook:some View {
        if let file=selected {
            VStack(alignment:.leading,spacing:12) {
                HStack {Text("QUICK LOOK").tracking(1.4);Spacer();if loading {ProgressView().controlSize(.small)}}.font(.system(size:10,weight:.medium)).foregroundStyle(.secondary)
                ScrollView {
                    VStack(alignment:.leading,spacing:16) {
                        if let uri=preview["image"] as? String,uri.hasPrefix("data:image/jpeg;base64,"),let data=Data(base64Encoded:String(uri.dropFirst(23))),let image=NSImage(data:data) {
                            Image(nsImage:image).resizable().scaledToFit().frame(maxWidth:.infinity,maxHeight:230).clipShape(RoundedRectangle(cornerRadius:10))
                        } else {
                            Image(nsImage:file.icon).resizable().frame(width:56,height:56).padding(.top,16).frame(maxWidth:.infinity)
                        }
                        VStack(alignment:.leading,spacing:6) {
                            Text(file.name).font(.system(size:17,weight:.semibold)).textSelection(.enabled)
                            Text(file.path).font(.system(size:11)).foregroundStyle(.secondary).lineLimit(3).textSelection(.enabled)
                            Text("\(file.typeLabel) · \(file.size) · \(model.deviceName(file.device))").font(.system(size:11)).foregroundStyle(.secondary)
                        }
                        if let coverage=preview["coverage"] as? String,!coverage.isEmpty {Text(coverage).font(.system(size:10)).foregroundStyle(gold)}
                        if !previewError.isEmpty {Text(previewError).font(.system(size:12)).foregroundStyle(.secondary)}
                        if let text=preview["text"] as? String,!text.isEmpty {
                            Text(text).font(.system(size:12)).lineSpacing(4).textSelection(.enabled).frame(maxWidth:.infinity,alignment:.leading)
                        } else if !file.detail.isEmpty {Text(file.detail).font(.system(size:12)).lineSpacing(4).foregroundStyle(.secondary)}
                    }
                }
                HStack(spacing:10) {
                    Button("Open") {openSelected()}.buttonStyle(.borderedProminent).tint(gold).foregroundStyle(.black)
                    Button("Show in Retriever") {(dismissAfterOpen ?? dismiss)();model.revealSelected()}.buttonStyle(.bordered).disabled(selected==nil)
                }.controlSize(.regular)
            }.padding(22)
        } else {
            VStack(spacing:12) {
                Image(systemName:"rectangle.inset.filled.and.person.filled").font(.system(size:30,weight:.ultraLight)).foregroundStyle(.secondary)
                Text("A little more context.").font(.system(size:15,weight:.medium))
                Text("Select a result to preview it here.").font(.system(size:12)).foregroundStyle(.secondary)
            }.frame(maxWidth:.infinity,maxHeight:.infinity)
        }
    }
    private func openSelected() {
        guard model.canOpenSelected,let file=selected else {model.message="Open this file on its source device. Remote opening is not available yet.";return}
        if file.kind=="folder" {(dismissAfterOpen ?? dismiss)();model.revealSelected();return}
        if NSWorkspace.shared.open(URL(fileURLWithPath:file.path)) {(dismissAfterOpen ?? dismiss)()} else {model.message="macOS could not open this file."}
    }
    private func move(_ amount:Int) {
        guard !model.files.isEmpty else {return}
        let index=model.files.firstIndex(where:{$0.id==selected?.id}) ?? (amount>0 ? -1 : model.files.count)
        model.selection=[model.files[max(0,min(model.files.count-1,index+amount))].id]
    }
    private func loadPreview() async {
        preview=[:];previewError="";loading=false
        guard model.quickActive,let file=selected else {return}
        loading=true
        do {
            try await Task.sleep(for:.milliseconds(160))
            var result:[String:Any]=[:]
            for attempt in 0..<5 {
                result=try await model.request("workspace/preview",body:["device":file.device,"path":file.path,"visual":true])
                if result["visual_pending"] as? Bool != true {break}
                try await Task.sleep(for:.milliseconds(300+attempt*150))
            }
            try Task.checkCancellation()
            guard selected?.id==file.id else {return}
            preview=result
            if result["offline"] as? Bool == true {previewError="Source offline · showing indexed text"}
            else if let note=result["visual_error"] as? String {previewError=note}
            loading=false
        } catch is CancellationError {} catch {
            guard !Task.isCancelled,selected?.id==file.id else {return}
            previewError=error.localizedDescription;loading=false
        }
    }
}

private struct QuickResultRow:View {
    let file:FileItem;let selected:Bool;let gold:Color
    @State private var hovered=false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    var body:some View {
        HStack(spacing:12) {
            Image(nsImage:file.icon).resizable().frame(width:28,height:28)
            VStack(alignment:.leading,spacing:4) {
                Text(file.name).font(.system(size:13,weight:.medium)).lineLimit(1)
                Text(file.detail.isEmpty ? file.path : file.detail).font(.system(size:11)).foregroundStyle(.secondary).lineLimit(1)
            }.frame(maxWidth:.infinity,alignment:.leading)
            if selected {Image(systemName:"return").font(.system(size:11)).foregroundStyle(gold)}
        }.padding(.horizontal,12).frame(height:62)
        .background(RoundedRectangle(cornerRadius:10).fill(selected ? gold.opacity(0.17) : .white.opacity(hovered ? 0.055 : 0)))
        .overlay(RoundedRectangle(cornerRadius:10).strokeBorder(selected ? gold.opacity(0.2) : .clear))
        .onHover {hovered=$0}.animation(reduceMotion ? nil : .easeOut(duration:0.12),value:hovered)
    }
}

final class QuickSearchPanel:NSPanel {
    override var canBecomeKey:Bool {true}
    override var canBecomeMain:Bool {false}
}

@MainActor final class QuickOverlayController {
    let model:WorkspaceModel
    let panel:QuickSearchPanel
    private var previousApp:NSRunningApplication?
    private var generation=0
    private var monitor:Any?
    private var outsideMonitor:Any?
    private var resignObserver:NSObjectProtocol?
    private var ready=false
    private var starting=false
    private var shown=false
    init(model:WorkspaceModel,showExplorer:@escaping () -> Void) {
        self.model=model
        panel=QuickSearchPanel(contentRect:NSRect(x:0,y:0,width:940,height:580),styleMask:[.borderless,.nonactivatingPanel],backing:.buffered,defer:false)
        panel.title="Retriever Quick Search"
        panel.isFloatingPanel=true;panel.level = .floating;panel.isOpaque=false;panel.backgroundColor = .clear
        panel.hasShadow=true;panel.hidesOnDeactivate=false;panel.isMovableByWindowBackground=true
        panel.collectionBehavior=[.canJoinAllSpaces,.fullScreenAuxiliary,.transient,.ignoresCycle]
        panel.isReleasedWhenClosed=false
        let hosting=NSHostingView(rootView:QuickOverlayView(model:model,dismiss:{},showExplorer:showExplorer))
        if #available(macOS 26.0,*) {
            let glass=NSGlassEffectView();glass.style = .regular;glass.cornerRadius=24;glass.tintColor=NSColor(white:0.09,alpha:0.12)
            glass.contentView=hosting;panel.contentView=glass
        } else {
            let glass=NSVisualEffectView();glass.material = .hudWindow;glass.blendingMode = .behindWindow;glass.state = .active
            glass.wantsLayer=true;glass.layer?.cornerRadius=24;glass.layer?.masksToBounds=true
            hosting.frame=glass.bounds;hosting.autoresizingMask=[.width,.height];glass.addSubview(hosting);panel.contentView=glass
        }
        hosting.rootView=QuickOverlayView(model:model,dismiss:{[weak self] in self?.hide()},showExplorer:{[weak self] in self?.hide(restoreFocus:false);showExplorer()},dismissAfterOpen:{[weak self] in self?.hide(restoreFocus:false)})
        monitor=NSEvent.addLocalMonitorForEvents(matching:[.keyDown,.leftMouseDown,.rightMouseDown,.otherMouseDown]) {[weak self] event in
            guard let self,self.shown else {return event}
            if event.type == .keyDown,event.keyCode==53 {self.hide();return nil}
            if event.type != .keyDown,event.window !== self.panel {
                let captured=self.generation
                DispatchQueue.main.async {[weak self] in guard let self,self.shown,self.generation==captured else{return};self.hide(restoreFocus:false)}
            }
            return event
        }
        // Mouse monitoring does not need Accessibility access. Do not reactivate
        // the previous app: the click must keep focus at its new destination.
        outsideMonitor=NSEvent.addGlobalMonitorForEvents(matching:[.leftMouseDown,.rightMouseDown,.otherMouseDown]) {[weak self] _ in
            Task { @MainActor [weak self] in guard let self,self.shown else{return};self.hide(restoreFocus:false) }
        }
        resignObserver=NotificationCenter.default.addObserver(forName:NSWindow.didResignKeyNotification,object:panel,queue:.main) {[weak self] _ in
            Task { @MainActor [weak self] in guard let self,self.shown else{return};self.hide(restoreFocus:false) }
        }
    }
    func toggle() {shown ? hide() : show()}
    func show() {
        shown=true
        generation+=1
        model.quickActive=true
        previousApp=NSWorkspace.shared.frontmostApplication
        let point=NSEvent.mouseLocation
        let screen=NSScreen.screens.first(where:{$0.frame.contains(point)}) ?? NSScreen.main
        if let frame=screen?.visibleFrame {
            let size=NSSize(width:min(940,frame.width-48),height:min(580,frame.height-64))
            panel.setFrame(NSRect(x:frame.midX-size.width/2,y:frame.midY-size.height/2+min(65,frame.height*0.06),width:size.width,height:size.height),display:false)
        }
        let reduced=NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
        panel.alphaValue=reduced ? 1 : 0
        panel.makeKeyAndOrderFront(nil)
        // Give Escape and typing an immediate key window even when invoked
        // from another application's menu bar or a global shortcut.
        panel.makeKey()
        // A non-activating key panel keeps the previous application available for Escape.
        if !reduced {
            NSAnimationContext.runAnimationGroup {context in context.duration=0.18;context.timingFunction=CAMediaTimingFunction(name:.easeOut);panel.animator().alphaValue=1}
            if let layer=panel.contentView?.layer {
                let scale=CABasicAnimation(keyPath:"transform.scale");scale.fromValue=0.975;scale.toValue=1;scale.duration=0.22;scale.timingFunction=CAMediaTimingFunction(name:.easeOut);layer.add(scale,forKey:"reveal")
            }
        }
        NotificationCenter.default.post(name:Notification.Name("RetrieverFocusQuickSearch"),object:nil)
        let focusGeneration=generation
        DispatchQueue.main.async {[weak self] in
            guard let self,self.shown,self.generation==focusGeneration else{return}
            self.panel.makeKeyAndOrderFront(nil)
            NotificationCenter.default.post(name:Notification.Name("RetrieverFocusQuickSearch"),object:nil)
        }
        if !ready && !starting {starting=true;Task {await model.start();let status=try? await model.request("status");ready=status?["runtime"] as? String == "rust";starting=false}}
        else if ready && !model.query.isEmpty {model.queryChanged()}
    }
    func hide(restoreFocus:Bool=true) {
        guard shown else{return}
        shown=false
        generation+=1;let captured=generation
        model.quickActive=false
        model.suspendQuickSearch()
        let finish: @MainActor @Sendable () -> Void = { [weak self] in
            guard let self,self.generation==captured else {return}
            self.panel.orderOut(nil);self.panel.alphaValue=1
            if restoreFocus,let previous=self.previousApp,previous.processIdentifier != ProcessInfo.processInfo.processIdentifier {previous.activate(options:[])}
        }
        if NSWorkspace.shared.accessibilityDisplayShouldReduceMotion {finish()}
        else {NSAnimationContext.runAnimationGroup({context in context.duration=0.12;panel.animator().alphaValue=0},completionHandler:{Task { @MainActor in finish() }})}
    }
    func dispose() {if let monitor {NSEvent.removeMonitor(monitor)};if let outsideMonitor {NSEvent.removeMonitor(outsideMonitor)};if let resignObserver {NotificationCenter.default.removeObserver(resignObserver)};monitor=nil;outsideMonitor=nil;resignObserver=nil;shown=false;model.suspendQuickSearch();panel.orderOut(nil)}
}
