// Exports public runtime properties from the user's hash-verified server JAR.
import com.google.gson.*;
import com.mojang.serialization.JsonOps;
import java.nio.file.*;
import net.minecraft.SharedConstants;
import net.minecraft.server.Bootstrap;
import net.minecraft.core.BlockPos;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.world.level.EmptyBlockGetter;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.phys.shapes.VoxelShape;
import net.minecraft.world.item.ItemStackTemplate;

public final class VanillaOracle {
    static JsonArray boxes(VoxelShape shape) {
        JsonArray result = new JsonArray();
        for (var box : shape.toAabbs()) {
            JsonArray bounds = new JsonArray();
            for (double v : new double[]{box.minX, box.minY, box.minZ, box.maxX, box.maxY, box.maxZ}) bounds.add(v);
            result.add(bounds);
        }
        return result;
    }
    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        JsonObject root = new JsonObject();
        root.addProperty("format", 1);
        root.addProperty("version", "26.3");
        root.addProperty("context", "EmptyBlockGetter; BlockPos.ZERO; empty CollisionContext");
        JsonArray states = new JsonArray();
        for (var state : Block.BLOCK_STATE_REGISTRY) {
            JsonObject row = new JsonObject();
            var block = state.getBlock();
            row.addProperty("id", Block.getId(state));
            row.addProperty("block", BuiltInRegistries.BLOCK.getKey(block).toString());
            row.addProperty("hardness", state.getDestroySpeed(EmptyBlockGetter.INSTANCE, BlockPos.ZERO));
            row.addProperty("blast_resistance", block.getExplosionResistance());
            row.addProperty("friction", block.getFriction());
            row.addProperty("light", state.getLightEmission());
            row.addProperty("requires_correct_tool", state.requiresCorrectToolForDrops());
            row.addProperty("can_occlude", state.canOcclude());
            row.addProperty("dynamic_shape", block.hasDynamicShape());
            row.addProperty("position_offset", state.hasOffsetFunction());
            try {
                row.add("outline", boxes(state.getShape(EmptyBlockGetter.INSTANCE, BlockPos.ZERO)));
                row.add("collision", boxes(state.getCollisionShape(EmptyBlockGetter.INSTANCE, BlockPos.ZERO)));
                row.add("occlusion", boxes(state.getOcclusionShape()));
            } catch (RuntimeException e) {
                row.addProperty("shape_error", e.getClass().getName());
            }
            states.add(row);
        }
        root.add("states", states);
        JsonObject remainders = new JsonObject();
        BuiltInRegistries.ITEM.keySet().stream().sorted().forEach(id -> {
            var remainder = BuiltInRegistries.ITEM.getValue(id).getCraftingRemainder();
            remainders.add(id.toString(), remainder == null ? JsonNull.INSTANCE : ItemStackTemplate.CODEC.encodeStart(JsonOps.INSTANCE, remainder).getOrThrow());
        });
        root.add("crafting_remainders", remainders);
        Files.writeString(Path.of(args[0]), new GsonBuilder().disableHtmlEscaping().create().toJson(root) + "\n");
    }
}
