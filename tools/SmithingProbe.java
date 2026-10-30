// Behavioral probe run against the user's pinned server; no game code is copied.
import com.mojang.serialization.JsonOps;
import net.minecraft.SharedConstants;
import net.minecraft.server.Bootstrap;
import net.minecraft.core.component.*;
import net.minecraft.network.chat.Component;
import net.minecraft.world.item.*;
import net.minecraft.world.item.crafting.*;
import java.util.Optional;

public final class SmithingProbe {
    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        var lookup = net.minecraft.data.registries.VanillaRegistries.createWorldLookup();
        var ops = lookup.createSerializationContext(JsonOps.INSTANCE);
        for (var item : new Item[]{Items.DIAMOND_PICKAXE, Items.NETHERITE_PICKAXE, Items.NETHERITE_UPGRADE_SMITHING_TEMPLATE, Items.NETHERITE_INGOT}) {
            var id = net.minecraft.core.registries.BuiltInRegistries.ITEM.getKey(item);
            var path = java.nio.file.Path.of(args[0], id.getNamespace(), "components/item", id.getPath() + ".json");
            var json = com.google.gson.JsonParser.parseString(java.nio.file.Files.readString(path)).getAsJsonObject();
            item.builtInRegistryHolder().bindComponents(DataComponentMap.CODEC.parse(ops, json.get("components")).getOrThrow());
        }
        var base = new ItemStack(Items.DIAMOND_PICKAXE);
        base.setDamageValue(321);
        base.set(DataComponents.MAX_DAMAGE, base.getMaxDamage());
        base.set(DataComponents.CUSTOM_NAME, Component.literal("Kept name"));
        base.remove(DataComponents.REPAIR_COST);
        var result = new ItemStackTemplate(Items.NETHERITE_PICKAXE);
        var recipe = new SmithingTransformRecipe(new Recipe.CommonInfo(true),
            Optional.of(Ingredient.of(Items.NETHERITE_UPGRADE_SMITHING_TEMPLATE)),
            Ingredient.of(Items.DIAMOND_PICKAXE), Optional.of(Ingredient.of(Items.NETHERITE_INGOT)), result);
        var input = new SmithingRecipeInput(new ItemStack(Items.NETHERITE_UPGRADE_SMITHING_TEMPLATE),base,new ItemStack(Items.NETHERITE_INGOT));
        System.out.println("base=" + ItemStack.CODEC.encodeStart(ops, base).getOrThrow());
        System.out.println("output=" + ItemStack.CODEC.encodeStart(ops, recipe.assemble(input)).getOrThrow());
        var override = DataComponentPatch.builder().set(DataComponents.DAMAGE, 7).build();
        System.out.println("override=" + ItemStack.CODEC.encodeStart(ops,
            TransmuteRecipe.createWithOriginalComponents(new ItemStackTemplate(Items.NETHERITE_PICKAXE, override),base)).getOrThrow());
    }
}
