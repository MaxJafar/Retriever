import SwiftUI

let insightPalette=[Color(red:0.9,green:0.74,blue:0.3),Color(red:0.56,green:0.71,blue:0.63),Color(red:0.61,green:0.67,blue:0.83),Color(red:0.75,green:0.61,blue:0.69)]
struct InsightCard<Content:View>:View {
    let title:String
    @ViewBuilder var content:Content
    var body:some View {VStack(alignment:.leading,spacing:10){Text(title.uppercased()).font(.system(size:10,weight:.medium)).foregroundStyle(.secondary);content;Spacer(minLength:0)}.padding(14).frame(maxWidth:.infinity,maxHeight:.infinity,alignment:.topLeading).background(.primary.opacity(0.035),in:RoundedRectangle(cornerRadius:10)).overlay(RoundedRectangle(cornerRadius:10).stroke(.primary.opacity(0.09),lineWidth:1))}
}
struct InsightRing:View {
    let fraction:Double?
    let value:String
    let caption:String
    var color=insightPalette[0]
    var ringLabel="left"
    @Environment(\.accessibilityReduceMotion) private var reducedMotion
    private func ring(_ size:CGFloat)->some View {
        ZStack{Circle().stroke(.primary.opacity(0.08),lineWidth:5);if let fraction{Circle().trim(from:0,to:min(1,max(0,fraction))).stroke(color,style:StrokeStyle(lineWidth:5,lineCap:.round)).rotationEffect(.degrees(-90))};VStack(spacing:1){Text(fraction.map{String(format:"%.0f%%",max(0,min(1,$0))*100)} ?? "—").font(.system(size:size>60 ? 16:12,weight:.semibold));Text(fraction==nil ? "unknown":ringLabel).font(.system(size:8)).foregroundStyle(.secondary)}}.frame(width:size,height:size).padding(3)
    }
    private func labels(_ size:CGFloat)->some View {VStack(alignment:.leading,spacing:5){Text(value).font(.system(size:size,weight:.semibold)).lineLimit(1);Text(caption).font(.system(size:10)).foregroundStyle(.secondary).fixedSize(horizontal:false,vertical:true)}}
    var body:some View {
        ViewThatFits(in:.horizontal){HStack(spacing:12){ring(70);labels(24)}.fixedSize(horizontal:true,vertical:false);HStack(spacing:8){ring(48);labels(17)}}.animation(reducedMotion ? nil:.easeOut(duration:0.2),value:fraction).accessibilityElement(children:.combine)
    }
}
struct CapacityOverview:View {
    @ObservedObject var model:WorkspaceModel
    private var total:Int64 {diskNumber((model.diskSummary["volume"] as? [String:Any])?["total"])}
    private var available:Int64 {diskNumber((model.diskSummary["volume"] as? [String:Any])?["available"])}
    private var totals:[String:Any] {model.diskRunning ? model.diskSummary["totals"] as? [String:Any] ?? [:]:model.diskCurrent}
    var body:some View {
        HStack(spacing:10){
            InsightCard(title:"Volume capacity"){InsightRing(fraction:total>0 ? Double(total-available)/Double(total):nil,value:total>0 ? diskBytes(available):"—",caption:total>0 ? "available of "+diskBytes(total):"Choose a folder to begin",ringLabel:"used")}
            InsightCard(title:"Selected folder"){
                Text(model.diskScan.isEmpty ? "—":diskBytes(diskNumber(totals["logical"]))).font(.system(size:26,weight:.semibold))
                Text("\(diskNumber(totals["files"])) files · \(diskBytes(diskNumber(totals["allocated"]))) allocated").font(.system(size:10)).foregroundStyle(.secondary)
                Text(model.diskData["previous_scan"] is String ? "Compare in Size changes":"First snapshot · scan again to compare").font(.system(size:10)).foregroundStyle(.tertiary)
            }
            InsightCard(title:"File type breakdown"){typeBars}
        }.frame(height:142).padding(.bottom,12)
    }
    private var typeBars:some View {
        let rows=model.diskData["types"] as? [[String:Any]] ?? []
        let sum=rows.reduce(Int64(0)){$0+diskNumber($1["logical"])}
        return VStack(spacing:7){
            if rows.isEmpty{Text("File types will appear after your first scan.").font(.system(size:11)).foregroundStyle(.secondary)}
            ForEach(Array(rows.prefix(4).enumerated()),id:\.offset){index,row in
                HStack(spacing:6){Text((row["extension"] as? String ?? "Other").uppercased()).frame(width:35,alignment:.leading);GeometryReader{g in ZStack(alignment:.leading){Capsule().fill(.primary.opacity(0.06));Capsule().fill(insightPalette[index]).frame(width:g.size.width*(sum>0 ? Double(diskNumber(row["logical"]))/Double(sum):0))}}.frame(height:4);Text(diskBytes(diskNumber(row["logical"]))).frame(width:62,alignment:.trailing)}.font(.system(size:10)).accessibilityElement(children:.combine)
            }
        }
    }
}
struct HealthOverview:View {
    @ObservedObject var model:WorkspaceModel
    private var state:String {model.healthDrive["status"] as? String ?? "Unknown"}
    private var color:Color {state=="Good" ? insightPalette[1]:state=="Bad" ? Color(red:0.89,green:0.61,blue:0.56):state=="Caution" ? insightPalette[0]:.secondary}
    private var life:Double? {(model.healthDrive["remaining_percent"] as? NSNumber)?.doubleValue}
    var body:some View {
        HStack(spacing:10){
            InsightCard(title:"Reported health"){Text(state).font(.system(size:26,weight:.semibold)).foregroundStyle(color);Text(model.healthDrive.isEmpty ? "No saved telemetry":model.healthDrive["present"] as? Bool==false ? "Saved reading · Disconnected":"Saved SMART assessment").font(.system(size:11)).foregroundStyle(.secondary)}
            InsightCard(title:"Temperature · saved history"){Text(healthValue(model.healthDrive["temperature_c"]," °C")).font(.system(size:26,weight:.semibold));TemperatureSparkline(samples:model.healthHistory).frame(height:30)}
            InsightCard(title:"Rated endurance"){InsightRing(fraction:life.map{$0/100},value:life.map{String(format:"%.0f%%",$0)} ?? "—",caption:life==nil ? "Not reported by this drive":"remaining · not a failure prediction",color:color)}
        }.frame(height:124)
    }
}
struct TemperatureSparkline:View {
    let samples:[[String:Any]]
    var body:some View {
        let valid=samples.compactMap{($0["temperature_c"] as? NSNumber)?.doubleValue}
        return GeometryReader{g in
            if valid.count<2 {Text("No measured trend yet").font(.system(size:10)).foregroundStyle(.secondary)}else{
                let low=(valid.min() ?? 0)-2;let high=(valid.max() ?? 1)+2;let first=(samples.first?["time"] as? NSNumber)?.doubleValue ?? 0;let last=(samples.last?["time"] as? NSNumber)?.doubleValue ?? 1
                Path{path in var connected=false;for sample in samples{guard let temp=(sample["temperature_c"] as? NSNumber)?.doubleValue,let time=(sample["time"] as? NSNumber)?.doubleValue else{connected=false;continue};let point=CGPoint(x:4+(time-first)/max(1,last-first)*(g.size.width-8),y:g.size.height-4-(temp-low)/(high-low)*(g.size.height-8));if connected{path.addLine(to:point)}else{path.move(to:point)};connected=true}}.stroke(insightPalette[0],lineWidth:2)
            }
        }.accessibilityLabel("Saved temperature history. Missing samples appear as gaps.")
    }
}
struct HealthThresholdPopover:View {
    @ObservedObject var model:WorkspaceModel
    @Environment(\.dismiss) private var dismiss
    @State private var temperature=70
    @State private var life=10
    var body:some View {VStack(alignment:.leading,spacing:14){Text("Health alerts").font(.headline);Stepper("Temperature: \(temperature) °C",value:$temperature,in:30...90);Stepper("Endurance remaining: \(life)%",value:$life,in:0...50);Text("Alert thresholds only. Drive firmware is untouched.").font(.caption).foregroundStyle(.secondary);Button("Save thresholds"){Task{await model.setHealthSettings(temperature:temperature,life:life);dismiss()}}}.padding(14).frame(width:290).onAppear{temperature=(model.healthSettings["temperature_c"] as? Int) ?? 70;life=(model.healthSettings["remaining_life_percent"] as? Int) ?? 10}}
}
