#!/usr/bin/env perl
# Strict ustar subset: bounded inspection before any archive member is extracted.
use strict;
use warnings;
my ($archive, $version) = @ARGV;
defined $version or die "archive and version required\n";
open my $in, '-|', 'gzip', '-dc', $archive or die "cannot decompress archive: $!\n";
binmode $in;
my ($count, $total, $root, %seen) = (0, 0);
sub block {
    my $buf = '';
    while (length($buf) < 512) {
        my $n = read($in, my $part, 512 - length($buf));
        defined $n && $n > 0 or die "truncated archive\n";
        $buf .= $part;
    }
    return $buf;
}
sub field { my ($s) = @_; $s =~ s/\0.*//s; return $s; }
sub octal {
    my ($s) = @_;
    $s =~ /\A[ 0-7]*[\0 ]*\z/ or die "invalid tar number\n";
    return oct($s);
}
while (1) {
    my $h = block();
    if ($h eq "\0" x 512) {
        block() eq "\0" x 512 or die "missing tar end marker\n";
        # Only bounded zero padding may follow, never another archive.
        my $padding = 0;
        while (my $n = read($in, my $tail, 512)) {
            $padding += $n;
            $padding <= 1048576 && $tail !~ /[^\0]/ or die "invalid archive trailer\n";
        }
        last;
    }
    ++$count <= 10000 or die "archive entry count rejected\n";
    my $stored = octal(substr($h, 148, 8));
    my $check = $h; substr($check, 148, 8) = ' ' x 8;
    unpack('%32C*', $check) == $stored or die "tar header checksum mismatch\n";
    substr($h, 257, 6) eq "ustar\0" or die "only POSIX ustar archives are accepted\n";
    my $type = substr($h, 156, 1);
    $type eq '0' || $type eq "\0" || $type eq '5' or die "links, extensions and special files rejected\n";
    my $name = field(substr($h, 0, 100));
    my $prefix = field(substr($h, 345, 155));
    $name = "$prefix/$name" if length $prefix;
    $name =~ s{/$}{} if $type eq '5';
    $name !~ m{\A/|[\x00-\x1f\x7f]|(?:\A|/)\.{1,2}(?:/|\z)|//} or die "unsafe archive path\n";
    my ($top) = split '/', $name;
    $top =~ /\Aparaco-\Q$version\E-[A-Za-z0-9._-]+\z/ or die "archive root/version mismatch\n";
    !defined($root) || $root eq $top or die "multiple archive roots\n";
    $root = $top;
    !$seen{$name}++ or die "duplicate archive path\n";
    my $size = octal(substr($h, 124, 12));
    $type ne '5' || $size == 0 or die "directory has data\n";
    $total += $size;
    $total <= 1073741824 or die "extraction size rejected\n";
    for (my $remaining = $size; $remaining > 0; $remaining -= 512) { block(); }
}
close $in or die "decompression failed\n";
defined($root) or die "empty archive\n";
print "$root\n";
