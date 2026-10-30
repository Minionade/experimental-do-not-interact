// Public-runtime behavioral checks against the user's verified 26.3 server.
import com.mojang.serialization.JsonOps;
import net.minecraft.SharedConstants;
import net.minecraft.server.Bootstrap;
import net.minecraft.core.component.*;
import net.minecraft.network.chat.Component;
import net.minecraft.world.item.*;
import net.minecraft.world.item.crafting.*;
import net.minecraft.advancements.predicates.MinMaxBounds;
import net.minecraft.core.registries.BuiltInRegistries;
import java.util.List;

public final class TransmuteProbe {
    static Item item(String id) { return BuiltInRegistries.ITEM.getValue(net.minecraft.resources.Identifier.parse(id)); }
    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion(); Bootstrap.bootStrap();
        var ops = net.minecraft.data.registries.VanillaRegistries.createWorldLookup().createSerializationContext(JsonOps.INSTANCE);
        for (var item : new Item[]{Items.BUNDLE,item("minecraft:pink_bundle"),item("minecraft:pink_dye"),Items.FILLED_MAP,Items.MAP,Items.DIAMOND}) {
            var id = BuiltInRegistries.ITEM.getKey(item);
            var path = java.nio.file.Path.of(args[0],id.getNamespace(),"components/item",id.getPath()+".json");
            var json = com.google.gson.JsonParser.parseString(java.nio.file.Files.readString(path)).getAsJsonObject();
            item.builtInRegistryHolder().bindComponents(DataComponentMap.CODEC.parse(ops,json.get("components")).getOrThrow());
        }
        var book = new CraftingRecipe.CraftingBookInfo(CraftingBookCategory.MISC,"probe");
        var dye = new TransmuteRecipe(new Recipe.CommonInfo(true),book,
            Ingredient.of(Items.BUNDLE,item("minecraft:pink_bundle")),Ingredient.of(item("minecraft:pink_dye")),
            MinMaxBounds.Ints.exactly(1),new TransmuteResult(item("minecraft:pink_bundle")),false);
        for (var source : new Item[]{Items.BUNDLE,item("minecraft:pink_bundle")}) {
            var base = new ItemStack(source);
            base.set(DataComponents.CUSTOM_NAME,Component.literal("Contents stay"));
            base.set(DataComponents.BUNDLE_CONTENTS,new net.minecraft.world.item.component.BundleContents(List.of(new ItemStackTemplate(Items.DIAMOND,7))));
            var input = CraftingInput.of(2,2,List.of(base,new ItemStack(item("minecraft:pink_dye"),64),ItemStack.EMPTY,ItemStack.EMPTY));
            if (dye.matches(input,null) != (source == Items.BUNDLE)) throw new AssertionError("same-color matching");
            var output = dye.assemble(input);
            if (!output.get(DataComponents.BUNDLE_CONTENTS).equals(base.get(DataComponents.BUNDLE_CONTENTS))) throw new AssertionError("lost contents");
            System.out.println(ItemStack.CODEC.encodeStart(ops,output).getOrThrow());
        }
        var clone = new TransmuteRecipe(new Recipe.CommonInfo(true),book,Ingredient.of(Items.FILLED_MAP),Ingredient.of(Items.MAP),
            MinMaxBounds.Ints.between(1,8),TransmuteResult.KEEP_INPUT_ITEM,true);
        var map = new ItemStack(Items.FILLED_MAP,3);
        map.set(DataComponents.MAP_ID,new net.minecraft.world.level.saveddata.maps.MapId(42));
        var input = CraftingInput.of(2,2,List.of(map,new ItemStack(Items.MAP,64),new ItemStack(Items.MAP,12),ItemStack.EMPTY));
        var output = clone.assemble(input);
        if (!clone.matches(input,null) || output.getCount()!=3 || !output.get(DataComponents.MAP_ID).equals(map.get(DataComponents.MAP_ID))) throw new AssertionError("map cloning");
        System.out.println(ItemStack.CODEC.encodeStart(ops,output).getOrThrow());
    }
}
