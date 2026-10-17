#if RETRIEVER_TESTING
using System.Text.Json.Nodes;
namespace Retriever.Native;

// In-memory dashboard navigation. No filesystem, process, or device access.
internal static class CapacityFixtures {
    static long N(JsonNode? value)=>long.TryParse(value?.ToString(),out var n)?n:0;
    internal static JsonNode Request(JsonNode fixture,string route,JsonNode? body){
        if(fixture["fixture_only"]?.GetValue<bool>()!=true)throw new InvalidDataException("Expected a synthetic capacity snapshot");
        if(route=="disk")return new JsonObject{["fixture_only"]=true,["scans"]=new JsonArray(fixture["scan"]!.DeepClone())};
        if(route!="disk/view")throw new InvalidOperationException("Synthetic capacity previews cannot scan folders.");
        var node=body?["node"]==null?1:N(body["node"]);var mode=body?["mode"]?.ToString()??"children";
        var result=fixture.DeepClone();var roots=fixture["entries"]!.AsArray().Select(e=>e!).ToArray();
        var current=node==1?fixture["current"]!:roots.FirstOrDefault(e=>N(e["id"])==node&&e["kind"]?.ToString()=="folder")??throw new InvalidDataException("Unknown synthetic folder");
        var entries=node==1?roots:Children(current).ToArray();
        if(mode!="children")entries=entries.SelectMany(e=>e["kind"]?.ToString()=="folder"?Children(e):new[]{e}).ToArray();
        if(mode=="older")entries=entries.Where(e=>N(e["fixture_age_days"])>=365).ToArray();
        entries=(mode=="growth"?entries.OrderByDescending(e=>N(e["delta"])):entries.OrderByDescending(e=>N(e["logical"]))).ToArray();
        result["current"]=current.DeepClone();result["entries"]=new JsonArray(entries.Select(e=>e.DeepClone()).ToArray());result["total"]=entries.Length;result["next_offset"]=null;
        if(node!=1||mode!="children"){
            var tiles=new JsonArray();double total=Math.Max(1,entries.Sum(e=>(double)N(e["logical"]))),x=0;
            foreach(var entry in entries){var tile=entry.DeepClone();double width=N(entry["logical"])/total;tile["rect"]=new JsonObject{["x"]=x,["y"]=0d,["width"]=width,["height"]=1d};tiles.Add(tile);x+=width;}
            result["treemap"]=tiles;
            result["types"]=new JsonArray(entries.GroupBy(e=>e["name"]!.ToString().Split('.').Last()).Select(g=>(JsonNode)new JsonObject{["extension"]=g.Key,["logical"]=g.Sum(e=>N(e["logical"]))}).ToArray());
        }
        return result;
    }
    static IEnumerable<JsonNode> Children(JsonNode folder){
        long total=N(folder["logical"]),id=N(folder["id"]);var names=id==2?new[]{"budget.xlsx","launch.pptx"}:new[]{"sample.mp4","support.docx"};
        for(int i=0;i<2;i++){long size=i==0?total*3/4:total-total*3/4;yield return new JsonObject{["id"]=id*100+i,["parent"]=id,["name"]=names[i],["path"]=folder["path"]+"/"+names[i],["kind"]="file",["logical"]=size,["allocated"]=size,["allocation_unknown"]=0,["files"]=1,["delta"]=i==0?1048576:-1048576,["fixture_age_days"]=i==0?420:30};}
    }
}
#endif
