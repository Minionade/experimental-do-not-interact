#version 330 core
in vec2 tex;
in float shade;
uniform sampler2D sprite;
out vec4 color;
void main(){color=texture(sprite,tex);if(color.a<0.1)discard;color.rgb*=shade;}
