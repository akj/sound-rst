CrazyMod 9 beta 10 adds ten factions to SoundRTS: traditional, technical, robotic, undead, elf, orc, elemental, wild, vermin and dark elf.
Its author's site is http://pragmapragma.free.fr/soundrts-crazymod/ and its files do not name the author.
This copy comes from SoundRTS 1.2-c4 (2018). It uses English names in place of the original's numbers and French ids, and keeps the author's German and French words. Its Spanish and Italian words are the later translations that the original game's repository keeps in misc/crazymod9beta10.

What the port changed, and why:
- The raise dead spells of the undead no longer list the empty "rien" unit. In 1.2-c4 it raised nothing and used no corpse; the port refuses a type without a class.
- The computer players lose the lines teleportation, send_soldiers_to_base, raise_dead, retaliate and clear, which neither 1.2-c4 nor the port runs.
- The computer players' misspelled goto_randum, tour_d_l_eau and troll__de_guerre are spelled as the commands and types they meant.
- The dark elf computer asks for its own workers on the branch that asked for ten wild workers, which it can never make.
- The recommended maps leave out pra12 and can5b, which do not exist.
- The style loses an unused earthquake entry that titled nothing, and the Spanish style loses three entries for types that CrazyMod 9 no longer has.
- The orc faction uses the game's own name "orc", with the author's German "Ork" and Spanish "orcos".
- Spells that hurt units or buildings now deal damage. In 1.2-c4 they missed those targets; the port's rules apply them as written.
- The vermin have computer players, easy and aggressive. The author left both empty, so a vermin computer only gathered.

To play it, choose Options, then Mods, then crazymod.
