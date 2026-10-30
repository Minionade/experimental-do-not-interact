#version 330 core
in vec3 color;
in vec3 normal;
uniform vec3 u_sun_direction;
layout(location = 0) out vec4 outColor;
void main() {
    float light = 0.20 + 0.80 * max(dot(normalize(normal), normalize(u_sun_direction)), 0.0);
    outColor = vec4(color * light, 1.0);
}
