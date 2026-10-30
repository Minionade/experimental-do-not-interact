#version 330 core
uniform sampler2D u_scene_color;
in vec2 texcoord;
out vec4 outColor;
void main() {
    vec3 color = texture(u_scene_color, texcoord).rgb;
    color = color / (color + vec3(0.65));
    outColor = vec4(pow(color, vec3(1.0 / 2.2)), 1.0);
}
